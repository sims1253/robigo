//! `robigo-rt [--vanilla] FILE | -e EXPR ...`: Rscript's top level over G3's
//! runtime (`docs/g3.md` §14.2, M1): the isolate made, the evaluator and the
//! C API installed, the base subset bound from the pinned GNU R sources
//! (`robigo_rt::base`), and the script run as Rscript runs it
//! (`robigo_rt::repl`). `robigo-rt --quiet [--no-save] -f FILE` runs it as
//! `R -f FILE` does instead, echoing each line after its prompt (R CMD
//! check's transcripts). `R_DEFAULT_PACKAGES` is ignored: there are no
//! other packages. Exit status 0, or 1 after "Execution halted"; 2 for a
//! usage or start-up failure.

use robigo_rt::repl::run_session;
use robigo_rtcore::OwnedIsolate;
use std::os::unix::ffi::OsStrExt;

const USAGE: &str = "Usage: robigo-rt [--vanilla] [options] file [args]\n   or: robigo-rt [--vanilla] [options] -e expr [-e expr2 ...] [args]\n   or: robigo-rt --quiet [options] -f file [--args args]\n";

fn main() {
    std::process::exit(run(std::env::args_os().skip(1).collect()));
}

fn run(args: Vec<std::ffi::OsString>) -> i32 {
    // Rscript's options: R's start-up options, then FILE or -e EXPR, then
    // the script's arguments (commandArgs(), not yet read by anything).
    // Or R's: start-up options with -f FILE among them, then --args.
    let mut exprs: Vec<Vec<u8>> = Vec::new();
    let mut file = None;
    // -f FILE or --file=FILE: None for stdin (`-f -`)
    let mut r_file: Option<Option<std::ffi::OsString>> = None;
    // R_Quiet and R_NoEcho (R_common_command_line)
    let (mut quiet, mut no_echo) = (false, false);
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
            b"--no-echo" | b"--slave" | b"-s" => (quiet, no_echo) = (true, true),
            b"--save" => {
                eprintln!("robigo-rt: --save (the workspace saved at the end) is not implemented yet");
                return 2;
            }
            // start-up options that change nothing robigo-rt has
            b"--vanilla" | b"--no-save" | b"--no-restore" | b"--no-environ" | b"--no-site-file" | b"--no-init-file"
            | b"--verbose" => {}
            _ if b.starts_with(b"--default-packages=") => {}
            // R: the rest are commandArgs()'s
            b"--args" if r_file.is_some() => break,
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
    // Rscript runs R with --no-echo; R -f echoes unless told not to
    let echo = r_file.is_some() && !no_echo;
    let mut stdin = false;
    if let Some(f) = r_file {
        if !exprs.is_empty() {
            // R: unix/system.c:Rf_initialize_R, R_Suicide (to stdout this
            // early)
            println!("Fatal error: cannot use -e with -f or --file");
            return 2;
        }
        if !quiet {
            eprintln!("robigo-rt: R's start-up greeting (-f without --quiet) is not implemented yet");
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

    let dir = match robigo_rt::base::source_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("robigo-rt: {e}");
            return 2;
        }
    };
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
        let home = std::env::var_os("R_HOME").map_or_else(|| std::path::PathBuf::from(".robigo/rt-home"), Into::into);
        match robigo_rt::base::stage0(cx, &home) {
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
    } else if let Err(e) = robigo_rt::base::load(cx, &dir) {
        eprintln!("robigo-rt: loading base: {e}");
        return 2;
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
#[allow(unsafe_code)]
fn install_sigint() {
    // SAFETY: the handler is async-signal-safe (one atomic store); signal()
    // is called before any other thread exists.
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
