//! `robigo-rt [--vanilla] FILE | -e EXPR ...`: Rscript's top level over G3's
//! runtime (`docs/g3.md` §14.2, M1): the isolate made, the evaluator and the
//! C API installed, the base subset bound from the pinned GNU R sources
//! (`robigo_rt::base`), and the script run as Rscript runs it
//! (`robigo_rt::repl`). `robigo-rt --quiet [--no-save] -f FILE` runs it as
//! `R -f FILE` does instead, echoing each line after its prompt (R CMD
//! check's transcripts), and with neither a FILE nor `-e`, a stdin that is
//! not a terminal is read the same way (`R --vanilla --no-echo < script`, as
//! GNU R's own build runs R). `R_DEFAULT_PACKAGES` is ignored: there are no
//! other packages. Exit status 0, or 1 after "Execution halted"; 2 for a
//! usage or start-up failure.

#![deny(unsafe_code)]

use robigo_rt::repl::run_session;
use robigo_rtcore::OwnedIsolate;
use std::os::unix::ffi::OsStrExt;

const USAGE: &str = "Usage: robigo-rt [--vanilla] [options] file [args]\n   or: robigo-rt [--vanilla] [options] -e expr [-e expr2 ...] [args]\n   or: robigo-rt --quiet [options] -f file [--args args]\n";

fn main() {
    let home = init_r_home();
    std::process::exit(run(std::env::args_os().skip(1).collect(), &home));
}

/// The `R_HOME` in effect, and where it came from.
enum RHome {
    /// The install's or the checkout's tree, with base's files.
    Given(std::path::PathBuf),
    /// Neither an install's tree nor the checkout's has base's files: the
    /// checkout's, which `scripts/rt-home.sh` makes.
    MissingDefault(std::path::PathBuf),
}

impl RHome {
    fn path(&self) -> &std::path::Path {
        match self {
            RHome::Given(p) | RHome::MissingDefault(p) => p,
        }
    }
}

/// The front end's `R_HOME` (m2m3-plan F7b): robigo-rt's own R_HOME tree,
/// as GNU R's `R` script exports its own `R_HOME_DIR` with `R_SHARE_DIR`,
/// `R_INCLUDE_DIR` and `R_DOC_DIR` under it. An installed robigo-rt's is
/// `<prefix>/lib/robigo` (the binary in `<prefix>/bin`); else the tree of
/// the checkout it was built from, `.robigo/rt-home` (`scripts/rt-home.sh`),
/// as an uninstalled GNU R's is its build directory. As the script does, an
/// `R_HOME` the environment has that is another (GNU R's own, say) is
/// ignored with GNU R's warning (to stdout: the script's `echo`), and the
/// three directories are always the tree's. Returns that `R_HOME`.
// R: scripts/R.sh.in
fn init_r_home() -> RHome {
    let home = default_r_home();
    if std::env::var_os("R_HOME").is_some_and(|h| !h.is_empty() && h != home.path().as_os_str()) {
        println!("WARNING: ignoring environment value of R_HOME");
    }
    let vars = [
        ("R_HOME", home.path().to_path_buf()),
        ("R_SHARE_DIR", home.path().join("share")),
        ("R_INCLUDE_DIR", home.path().join("include")),
        ("R_DOC_DIR", home.path().join("doc")),
    ];
    for (var, value) in vars {
        // SAFETY: the first thing main does, before any other thread
        // exists to read the environment.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var(var, value)
        };
    }
    home
}

/// robigo-rt's R_HOME tree when the environment names none: an install's,
/// else the build checkout's, each only when it has base's files.
/// `MissingDefault` with the checkout's when neither has: it is still
/// exported (so that `R.home()` names where the tree belongs), and what
/// reads the tree says it is missing ([`missing_tree`]).
fn default_r_home() -> RHome {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.parent()?.join("lib/robigo")))
        .filter(|h| has_base(h));
    if let Some(h) = installed {
        return RHome::Given(std::path::absolute(&h).unwrap_or(h));
    }
    // the build checkout, or the one `ROBIGO_RT_CHECKOUT` names (robigo's
    // own control, not GNU R's R_HOME: a relocated binary, a test)
    let checkout = match std::env::var_os("ROBIGO_RT_CHECKOUT").filter(|c| !c.is_empty()) {
        Some(c) => std::path::PathBuf::from(c),
        None => match option_env!("ROBIGO_WORKTREE") {
            Some(w) if !w.is_empty() => std::path::PathBuf::from(w),
            _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        },
    };
    let h = checkout.join(".robigo/rt-home");
    let h = std::path::absolute(&h).unwrap_or(h);
    if has_base(&h) { RHome::Given(h) } else { RHome::MissingDefault(h) }
}

/// Whether `h` holds base's tree: the file stage 0 reads first,
/// `library/base/R/base`.
fn has_base(h: &std::path::Path) -> bool {
    h.join("library/base/R/base").is_file()
}

/// The start-up error when robigo-rt's tree is not there, when stage 0 is
/// to read it: GNU R's script's "R_HOME ('...') not found" (it checks its
/// own home at start-up; robigo-rt's M1 loader reads no tree, so the check
/// waits for what does), with the way to make the tree.
// R: scripts/R.sh.in ("R_HOME may have moved, so check")
fn missing_tree(home: &RHome) -> Option<String> {
    match home {
        RHome::Given(_) => None,
        RHome::MissingDefault(p) => Some(format!(
            "R_HOME ('{}') not found: no installed tree (<prefix>/lib/robigo) has library/base/R/base, \
             and the checkout's is not made (scripts/rt-home.sh)",
            p.display()
        )),
    }
}

/// The front end over `args`, with `home` the `R_HOME` that
/// [`init_r_home`] has put in the environment.
fn run(args: Vec<std::ffi::OsString>, home: &RHome) -> i32 {
    // Rscript's options: R's start-up options, then FILE or -e EXPR, then
    // the script's arguments (commandArgs(), not yet read by anything).
    // Or R's: start-up options with -f FILE among them, then --args.
    let mut exprs: Vec<Vec<u8>> = Vec::new();
    let mut file = None;
    // -f FILE or --file=FILE: None for stdin (`-f -`)
    let mut r_file: Option<Option<std::ffi::OsString>> = None;
    // R_Quiet and R_NoEcho (R_common_command_line), and whether the save
    // action is given (SA_NOSAVE: --vanilla, --no-save, --no-echo)
    let (mut quiet, mut no_echo, mut save_given) = (false, false, false);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let b = a.as_encoded_bytes();
        match b {
            b"-e" => match it.next() {
                Some(e) => exprs.push(e.as_encoded_bytes().to_vec()),
                None => {
                    eprintln!("-e not followed by an expression");
                    return 2;
                }
            },
            // the R front end's `-f FILE` is `--file=FILE` for a FILE that
            // does not start with "-" (R: unix/R.sh.in)
            b"-f" => match it.next() {
                Some(f) if !f.is_empty() && !f.as_encoded_bytes().starts_with(b"-") => r_file = Some(Some(f)),
                _ => {
                    eprintln!("ERROR: option '-f' requires a filename argument");
                    return 1;
                }
            },
            // R: unix/system.c:Rf_initialize_R, R_INIT_TREAT_F (`-`: stdin)
            _ if b.starts_with(b"--file=") => {
                let f = std::ffi::OsStr::from_bytes(&b[b"--file=".len()..]).to_os_string();
                r_file = Some((f != "-").then_some(f));
            }
            b"--help" | b"-h" => {
                print!("{USAGE}");
                return 0;
            }
            // R: CommandLineArgs.c:R_common_command_line
            b"--quiet" | b"--silent" | b"-q" => quiet = true,
            b"--no-echo" | b"--slave" | b"-s" => (quiet, no_echo, save_given) = (true, true, true),
            b"--save" => {
                eprintln!("robigo-rt: --save (the workspace saved at the end) is not implemented yet");
                return 2;
            }
            // start-up options that change nothing robigo-rt has
            b"--vanilla" | b"--no-save" => save_given = true,
            b"--no-restore" | b"--no-environ" | b"--no-site-file" | b"--no-init-file" | b"--verbose" => {}
            _ if b.starts_with(b"--default-packages=") => {}
            // R: the rest are commandArgs()'s, whatever the mode (an
            // Rscript's FILE ends the options before it)
            b"--args" => break,
            _ if b.starts_with(b"-") && exprs.is_empty() && file.is_none() => {
                eprintln!("robigo-rt: unknown option '{}'", a.to_string_lossy());
                return 2;
            }
            _ if r_file.is_some() => {
                // R: unix/system.c:Rf_initialize_R, by R_ShowMessage (to
                // stdout this early)
                println!("ARGUMENT '{}' __ignored__\n", a.to_string_lossy());
            }
            _ => {
                if exprs.is_empty() && file.is_none() {
                    file = Some(a);
                }
                // the rest are the script's arguments
                break;
            }
        }
    }
    // R with neither a file nor -e (`R --vanilla --no-echo < script`, as GNU
    // R's own build runs R_EXE): stdin is the console, as `-f -` reads it,
    // when it is not a terminal (R_Interactive is R_isatty(0)); a terminal's
    // interactive console is not ported, so Rscript's usage stays there
    // R: unix/system.c:Rf_initialize_R
    if exprs.is_empty() && file.is_none() && r_file.is_none() {
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprint!("{USAGE}");
            return 2;
        }
        r_file = Some(None);
    }
    // R (-f, --file, stdin), not Rscript (a FILE or -e, with --no-echo)
    let r_mode = r_file.is_some();
    // Rscript runs R with --no-echo; R -f echoes unless told not to
    let echo = r_mode && !no_echo;
    let mut stdin = false;
    if let Some(f) = r_file {
        if !exprs.is_empty() {
            // R: unix/system.c:Rf_initialize_R, R_Suicide (to stdout this
            // early)
            println!("Fatal error: cannot use -e with -f or --file");
            return 2;
        }
        stdin = f.is_none();
        file = f;
    }
    let src = if !exprs.is_empty() {
        // R -e: the expressions, one per line
        let mut s = Vec::new();
        for e in exprs {
            s.extend_from_slice(&e);
            s.push(b'\n');
        }
        s
    } else if stdin {
        // non-interactive R needs the save action named, which a FILE
        // implies (`if (ifp && ...) SaveAction = SA_NOSAVE`) and stdin does
        // not: checked before anything is read (an open pipe does not hold
        // it up)
        // R: unix/system.c:Rf_initialize_R, R_Suicide
        if !save_given {
            eprintln!("Fatal error: you must specify '--save', '--no-save' or '--vanilla'");
            return 2;
        }
        if !quiet {
            eprintln!("robigo-rt: R's start-up greeting (R without --quiet) is not implemented yet");
            return 2;
        }
        // `-f -`: the console input is stdin
        let mut s = Vec::new();
        if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut s) {
            eprintln!("robigo-rt: reading stdin: {e}");
            return 2;
        }
        s
    } else if let Some(f) = file {
        match std::fs::read(&f) {
            Ok(s) => s,
            Err(e) => {
                // R: unix/system.c:Rf_initialize_R, R_INIT_TREAT_F's
                // R_Suicide (to stdout this early)
                println!("Fatal error: cannot open file '{}': {}", f.to_string_lossy(), io_reason(&e));
                return 2;
            }
        }
    } else {
        eprint!("{USAGE}");
        return 2;
    };
    // setup_Rmainloop's greeting, unless R_Quiet (a FILE's open error, at
    // the options, comes first)
    if r_mode && !stdin && !quiet {
        eprintln!("robigo-rt: R's start-up greeting (R without --quiet) is not implemented yet");
        return 2;
    }

    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    // R_NoEcho before InitOptions, whose `echo` is its negation
    cx.set_no_echo(!echo);
    robigo_rt::install(cx);
    // `.Call` and `dyn.load`, before base binds its stubs
    robigo_rtapi::install(cx);
    // L2's stage 0 (base evaluated from source, ROBIGO_RT_BOOT=stage0, from
    // R_HOME's library/base/R/base: scripts/rt-home.sh) beside M1's loader
    // until L5 boots from the databases
    if std::env::var_os("ROBIGO_RT_BOOT").is_some_and(|v| v == "stage0") {
        if let Some(e) = missing_tree(home) {
            eprintln!("robigo-rt: {e}");
            return 2;
        }
        match robigo_rt::base::stage0(cx, home.path()) {
            Ok(st) => {
                if std::env::var_os("ROBIGO_BOOT_VERBOSE").is_some() {
                    for (line, m) in &st.failed {
                        eprintln!("robigo-rt: stage 0: line {line}: {m}");
                    }
                    eprintln!("robigo-rt: stage 0: {} expressions, {} failed", st.exprs, st.failed.len());
                }
            }
            Err(e) => {
                eprintln!("robigo-rt: stage 0: {e}");
                return 2;
            }
        }
    } else {
        // (the pinned sources M1's loader reads: stage 0 reads R_HOME's)
        let dir = match robigo_rt::base::source_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("robigo-rt: {e}");
                return 2;
            }
        };
        if let Err(e) = robigo_rt::base::load(cx, &dir) {
            eprintln!("robigo-rt: loading base: {e}");
            return 2;
        }
    }
    // the isolate's console is the process's stdout and stderr
    // E9's ROBIGO_FAIL_ALLOC_AT counts the script's allocations, not base's
    cx.arm_fail_alloc();
    install_sigint();
    // end_Rmainloop's R_CleanUp(SA_DEFAULT, 0, 1) runs .Last; after an
    // error, check_session_exit's R_CleanUp(SA_NOSAVE, 1, 0) does not
    // (end_Rmainloop's newline unless R_NoEcho, then .Last: run_session)
    let ok = run_session(cx, &src);
    if !ok {
        // R: main.c:check_session_exit
        cx.reprintf(b"Execution halted\n");
    }
    // R_CleanUp: the exit finalizers, then the warnings .Last left
    // R: unix/sys-std.c:Rstd_CleanUp
    robigo_rt::fin::run_exit_finalizers(cx);
    robigo_rt::repl::print_warnings(cx);
    cx.flush_console();
    if ok { 0 } else { 1 }
}

/// GNU R's `handleInterrupt` (main.c), the SIGINT handler:
/// `R_interrupts_pending = 1`, which the next poll takes.
extern "C" fn handle_interrupt(_sig: libc::c_int) {
    robigo_rtcore::note_sigint();
}

/// setup_Rmainloop's `if (signal(SIGINT, handleInterrupt) == SIG_IGN)
/// signal(SIGINT, SIG_IGN)`: SIGINT is the user interrupt, unless the
/// process started with it ignored.
// R: main.c:setup_Rmainloop
fn install_sigint() {
    // SAFETY: the handler is async-signal-safe (one atomic store); signal()
    // is called before any other thread exists.
    #[allow(unsafe_code)]
    unsafe {
        if libc::signal(libc::SIGINT, handle_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t)
            == libc::SIG_IGN
        {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
    }
}

/// `strerror` for the usual cases, as GNU R prints it.
fn io_reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        _ => e.to_string(),
    }
}

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

#[cfg(test)]
mod tests {
    use super::*;

    /// The unmade checkout's message (no installed tree either), which an
    /// integration test cannot reach: it depends on the machine's trees.
    #[test]
    fn a_missing_default_tree_says_how_to_make_it() {
        let h = std::path::PathBuf::from("/nonexistent/robigo/.robigo/rt-home");
        assert_eq!(
            missing_tree(&RHome::MissingDefault(h.clone())).as_deref(),
            Some(
                "R_HOME ('/nonexistent/robigo/.robigo/rt-home') not found: no installed tree \
                 (<prefix>/lib/robigo) has library/base/R/base, and the checkout's is not made (scripts/rt-home.sh)"
            )
        );
        assert_eq!(missing_tree(&RHome::Given(h)), None);
    }
}
