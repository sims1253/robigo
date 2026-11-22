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

mod renviron;

use robigo_rt::repl::run_session;

/// The commit this binary was built from (build.rs's `ROBIGO_RT_COMMIT`), as
/// a marker in the file that scripts/rt-builder.sh reads, so that a tree
/// this binary builds records what built it. Nothing prints it. "unknown"
/// when the build script that ran did not set it (checkouts sharing a
/// target directory share its fingerprints, so an older checkout's build
/// script can be the one cargo ran): rt-builder.sh then refuses the binary
/// rather than record a wrong commit.
#[used]
static BUILD_COMMIT: [u8; 64] = build_commit(match option_env!("ROBIGO_RT_COMMIT") {
    Some(c) => c,
    None => "unknown",
});

/// `robigo-rt-commit:<commit>\n`, then NULs, in 64 bytes (a 40-digit
/// commit fits; a longer value is cut).
const fn build_commit(commit: &str) -> [u8; 64] {
    let mut out = [0u8; 64];
    let head = b"robigo-rt-commit:";
    let c = commit.as_bytes();
    let mut i = 0;
    while i < head.len() {
        out[i] = head[i];
        i += 1;
    }
    let mut j = 0;
    while j < c.len() && i < 63 {
        out[i] = c[j];
        i += 1;
        j += 1;
    }
    out[i] = b'\n';
    out
}
use robigo_rtcore::OwnedIsolate;
use std::os::unix::ffi::OsStrExt;

const USAGE: &str = "Usage: robigo-rt [--vanilla] [options] file [args]\n   or: robigo-rt [--vanilla] [options] -e expr [-e expr2 ...] [args]\n   or: robigo-rt --quiet [options] -f file [--args args]\n";

fn main() {
    // an unknown boot is a mistake, not M1's loader by default; read once,
    // before anything (an Renviron's ROBIGO_RT_BOOT changes nothing)
    match boot_checked() {
        Ok(b) => {
            let _ = BOOT.set(b);
        }
        Err(m) => {
            eprintln!("robigo-rt: {m}");
            std::process::exit(2);
        }
    }
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
    // the source tree (scripts/rt-home.sh), or with ROBIGO_RT_BOOT=rdb the
    // one with base's database (scripts/rt-home-db.sh)
    let rdb = boot() == Boot::Rdb;
    let h = checkout.join(if rdb { ".robigo/rt-home-db" } else { ".robigo/rt-home" });
    let h = std::path::absolute(&h).unwrap_or(h);
    let whole = if rdb { has_base_db(&h) } else { has_base(&h) };
    if whole { RHome::Given(h) } else { RHome::MissingDefault(h) }
}

/// What `R_common_command_line`'s start-up options leave for
/// `Rf_initialize_R` and `setup_Rmainloop`: whether the site and user
/// Renviron files are read (`NoRenviron`), the site and user profiles
/// (`LoadSiteFile`, `LoadInitFile`), and the saved workspace restored
/// (`RestoreAction`).
#[derive(Clone, Copy)]
struct Startup {
    environ: bool,
    site_file: bool,
    init_file: bool,
    restore: bool,
}

/// `R_ExpandFileName` as it is before R runs (no readline yet: the unix
/// version): `~` and `~/...` from `HOME` (else the password database),
/// `~user/...` from the password database, anything else as it is.
// R: sys-unix.c:R_ExpandFileName_unix
fn expand_tilde(s: &[u8]) -> Vec<u8> {
    if s.first() != Some(&b'~') {
        return s.to_vec();
    }
    let end = s.iter().position(|&b| b == b'/').unwrap_or(s.len());
    let user = &s[1..end];
    let home = if user.is_empty() {
        std::env::var_os("HOME")
            .map(|h| h.into_encoded_bytes())
            .filter(|h| !h.is_empty())
            .or_else(|| robigo_rtcore::clib::passwd_home(None))
    } else {
        robigo_rtcore::clib::passwd_home(Some(user))
    };
    match home {
        Some(mut h) => {
            h.extend_from_slice(&s[end..]);
            h
        }
        None => s.to_vec(),
    }
}

/// How base boots (`ROBIGO_RT_BOOT`): INTERIM(L5) M1's lazy loader of the
/// pinned sources (unset), or GNU R's boot, `R_ReplFile` of R_HOME's
/// `library/base/R/base`: of the checkout's source tree (`stage0`), or of
/// its tree with base's database (`rdb`). An installed tree is what it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boot {
    M1,
    Stage0,
    Rdb,
}

/// `ROBIGO_RT_BOOT`'s boot; unset or empty is M1's loader, any other
/// value than `stage0` or `rdb` an error (a typo must not boot M1's
/// loader silently).
fn boot_checked() -> Result<Boot, String> {
    match std::env::var_os("ROBIGO_RT_BOOT") {
        None => Ok(Boot::M1),
        Some(v) if v.is_empty() => Ok(Boot::M1),
        Some(v) if v == "stage0" => Ok(Boot::Stage0),
        Some(v) if v == "rdb" => Ok(Boot::Rdb),
        Some(v) => {
            Err(format!("ROBIGO_RT_BOOT is '{}': it is 'stage0', 'rdb' or unset (M1's loader)", v.to_string_lossy()))
        }
    }
}

/// The boot `main` read at the start ([`boot_checked`]).
static BOOT: std::sync::OnceLock<Boot> = std::sync::OnceLock::new();

/// The boot `main` read at the start.
fn boot() -> Boot {
    *BOOT.get().expect("robigo: the boot is read first in main")
}

/// Whether `h` holds base's tree: the file stage 0 reads first,
/// `library/base/R/base`.
fn has_base(h: &std::path::Path) -> bool {
    h.join("library/base/R/base").is_file()
}

/// Whether the checkout's database tree is whole: its base file
/// (baseloader.R) and the database it loads.
fn has_base_db(h: &std::path::Path) -> bool {
    has_base(h) && ["base.rdb", "base.rdx"].iter().all(|f| h.join("library/base/R").join(f).is_file())
}

/// The start-up error when robigo-rt's tree is not there, when stage 0 is
/// to read it: GNU R's script's "R_HOME ('...') not found" (it checks its
/// own home at start-up; robigo-rt's M1 loader reads no tree, so the check
/// waits for what does), with the way to make the tree.
// R: scripts/R.sh.in ("R_HOME may have moved, so check")
fn missing_tree(home: &RHome, boot: Boot) -> Option<String> {
    match home {
        RHome::Given(_) => None,
        RHome::MissingDefault(p) => Some(format!(
            "R_HOME ('{}') not found: no installed tree (<prefix>/lib/robigo) has library/base/R/base, \
             and the checkout's is not made ({})",
            p.display(),
            if boot == Boot::Rdb { "scripts/rt-home-db.sh" } else { "scripts/rt-home.sh" }
        )),
    }
}

/// The front end over `args`, with `home` the `R_HOME` that
/// [`init_r_home`] has put in the environment.
fn run(args: Vec<std::ffi::OsString>, home: &RHome) -> i32 {
    // Rf_initialize_R's system Renviron, before the options are read
    // (INTERIM(L5): GNU R's boot only; M1's loader gives the options their
    // values itself)
    if boot() != Boot::M1 {
        renviron::process_system_renviron(home.path());
    }
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
    // --interactive (force_interactive); --no-readline is robigo-rt's only
    // console (it has no readline)
    let mut force_interactive = false;
    // R_common_command_line's NoRenviron, LoadSiteFile, LoadInitFile and
    // RestoreAction (--vanilla is all four, with --no-save)
    let mut st = Startup { environ: true, site_file: true, init_file: true, restore: true };
    // the restore action as the options leave it, in their order (None:
    // the mode's, R's restore or Rscript's --no-restore, which Rscript puts
    // before the user's options)
    let mut restore: Option<bool> = None;
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
            b"--vanilla" => {
                save_given = true;
                st = Startup { environ: false, site_file: false, init_file: false, restore: false };
                restore = Some(false);
            }
            b"--no-save" => save_given = true,
            b"--no-environ" => st.environ = false,
            b"--no-site-file" => st.site_file = false,
            b"--no-init-file" => st.init_file = false,
            b"--no-restore" | b"--no-restore-data" => restore = Some(false),
            b"--restore" => restore = Some(true),
            b"--verbose" => {}
            b"--interactive" => force_interactive = true,
            b"--no-readline" => {}
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
    // R with neither a file nor -e: stdin is the console. R_Interactive is
    // TRUE but for -f, --file and -e, then `R_Interactive && (force_interactive
    // || R_isatty(0))`: `R --vanilla --no-echo < script`, as GNU R's own build
    // runs R_EXE, is not interactive; a terminal, or --interactive, is (the
    // console without readline, as `--no-readline`)
    // R: unix/system.c:Rf_initialize_R
    let mut interactive = false;
    if exprs.is_empty() && file.is_none() && r_file.is_none() {
        interactive = force_interactive || std::io::IsTerminal::is_terminal(&std::io::stdin());
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
    // Rscript runs R with --no-restore before the user's options (a FILE or
    // -e); R restores unless told not to
    st.restore = restore.unwrap_or(r_mode);
    // Rf_initialize_R's site and user Renviron, after the options and
    // before any input is read, unless --no-environ
    if boot() != Boot::M1 && st.environ {
        renviron::process_site_renviron(home.path());
        renviron::process_user_renviron(expand_tilde);
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
        if !interactive && !save_given {
            eprintln!("Fatal error: you must specify '--save', '--no-save' or '--vanilla'");
            return 2;
        }
        // INTERIM: interactive R's default SaveAction is SA_SAVEASK, whose
        // question at the end is the session's clean-up (L's quit), not here yet
        if interactive && !save_given {
            eprintln!(
                "robigo-rt: interactive R without --no-save or --vanilla (the 'Save workspace image?' question) is not implemented yet"
            );
            return 2;
        }
        // the console input is stdin, read as the session goes (below)
        Vec::new()
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
    // R_Quiet: --quiet, or --no-echo (which Rscript passes)
    let r_quiet = quiet || !r_mode;

    // setup_Rmainloop's setlocale(<category>, "") calls and R_check_locale:
    // the isolate's locale is the environment's (the process's stays "C")
    let env_loc = robigo_rtcore::clib::env_locale();
    let utf8 = if env_loc.codeset.eq_ignore_ascii_case(b"UTF-8") {
        true
    } else if env_loc.codeset == b"ANSI_X3.4-1968" {
        false
    } else {
        eprintln!(
            "robigo-rt: a locale whose codeset is '{}' is not implemented yet",
            String::from_utf8_lossy(&env_loc.codeset)
        );
        return 2;
    };
    let mut iso = OwnedIsolate::new();
    // The process environment is this isolate's to write (Sys.setenv,
    // Sys.unsetenv), as it is GNU R's.
    // SAFETY: EnvWrites::assume's contract holds for this binary. It runs
    // this one isolate, on the main thread, and starts no threads of its
    // own (the rt crates spawn none), so every write happens on that
    // thread, and so do its own C readers of the environment outside clib,
    // which are therefore sequential with the writes: RC_fopen's `fopen`
    // with a `ccs=` mode reads GCONV_PATH (rtapi sysutils.rs); `dlerror`
    // reads LANGUAGE through gettext, and `dlopen`, `dlclose` and `dlsym`
    // run a library's constructors, destructors and IFUNC resolvers (rtapi
    // rdynload.rs). On a glibc older than 2.41 it also assumes, as the
    // contract asks, that no thread started by loaded native code reads the
    // environment while this isolate may write: GNU R's rule that R's API
    // belongs to the one R thread does not cover such threads, so this is
    // an assumption about the packages it loads, not a consequence.
    #[allow(unsafe_code)]
    iso.grant_env_writes(unsafe { robigo_rtcore::clib::EnvWrites::assume() });
    let cx = iso.cx();
    cx.set_locale(robigo_rtcore::Locale { utf8, collate_c: env_loc.collate_c });
    cx.set_locale_names(env_loc.names);
    // R_NoEcho before InitOptions, whose `echo` is its negation
    cx.set_no_echo(!echo);
    robigo_rt::install(cx);
    // R_Interactive, as Rf_initialize_R left it, before base and the profiles
    robigo_rtbuiltins::sysutils::set_interactive(cx, interactive);
    // `.Call` and `dyn.load`, before base binds its stubs
    robigo_rtapi::install(cx);
    // GNU R's boot of base (R_ReplFile of R_HOME's library/base/R/base: the
    // source tree's, ROBIGO_RT_BOOT=stage0, or baseloader.R's ..lazyLoad
    // from base.rdb, ROBIGO_RT_BOOT=rdb) beside M1's loader until L5's bar
    if boot() != Boot::M1 {
        if let Some(e) = missing_tree(home, boot()) {
            eprintln!("robigo-rt: {e}");
            return 2;
        }
        match robigo_rt::base::stage0(cx, home.path()) {
            Ok(boot_st) => {
                // setup_Rmainloop after the system profile: .OptRequireMethods,
                // the site and user profiles, .RData, .First, .First.sys; an
                // error there halts R (check_session_exit: R_CleanUp(SA_NOSAVE,
                // 1, 0), no .Last)
                let a = robigo_rt::base::AfterProfile {
                    site_file: st.site_file,
                    init_file: st.init_file,
                    restore: st.restore,
                    quiet: r_quiet,
                };
                if !robigo_rt::base::after_profile(cx, home.path(), a) {
                    cx.reprintf(b"Execution halted\n");
                    robigo_rt::fin::run_exit_finalizers(cx);
                    robigo_rt::repl::print_warnings(cx);
                    cx.flush_console();
                    return 1;
                }
                let st = boot_st;
                if std::env::var_os("ROBIGO_BOOT_VERBOSE").is_some() {
                    for (line, m) in &st.failed {
                        eprintln!("robigo-rt: stage 0: line {line}: {m}");
                    }
                    eprintln!(
                        "robigo-rt: stage 0: {} expressions, {} failed; {} names.c entries not implemented",
                        st.exprs,
                        st.failed.len(),
                        st.stubs
                    );
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
        // INTERIM(L5): M1's loader runs no profiles; the greeting where
        // setup_Rmainloop prints it, after base and before the session
        if !r_quiet {
            robigo_rt::base::print_greeting(cx);
        }
    }
    // the setlocale failures setup_Rmainloop deferred, as warnings at its
    // end (quiet when neither LANG nor LC_ALL is set and not empty), then
    // "During startup - " and the warnings
    // R: main.c:setup_Rmainloop
    let set = |n: &str| std::env::var_os(n).is_some_and(|v| !v.is_empty());
    if (set("LANG") || set("LC_ALL")) && !env_loc.failed.is_empty() {
        for c in &env_loc.failed {
            let _ = cx.warning(&format!("Setting {c} failed, using \"C\"\n"));
        }
        if !cx.collected_warnings().is_empty() {
            cx.reprintf(b"During startup - ");
            robigo_rt::repl::print_warnings(cx);
        }
    }
    // setup_Rmainloop's last step, GNU R's boot: R_init_jit_enabled (the
    // compiler loaded when the JIT is on); M1's loader set the JIT up when
    // the evaluator was installed
    if boot() != Boot::M1 {
        match robigo_rt::base::init_jit(cx) {
            robigo_rt::base::JitStart::Done => {}
            robigo_rt::base::JitStart::Halt => {
                cx.reprintf(b"Execution halted\n");
                robigo_rt::fin::run_exit_finalizers(cx);
                robigo_rt::repl::print_warnings(cx);
                cx.flush_console();
                return 1;
            }
            robigo_rt::base::JitStart::Suicide => {
                // R_CleanUp(SA_SUICIDE, 2, 0)
                robigo_rt::fin::run_exit_finalizers(cx);
                cx.flush_console();
                return 2;
            }
        }
    }
    // the boot is over (setup_Rmainloop's last step, R_init_jit_enabled, ran
    // above: the compiler's namespace is loaded when the JIT is on). With
    // ROBIGO_GC_FREEZE=1 (off by default) this is the boot boundary: the
    // first collection after it freezes what the boot left reachable, so no
    // later collection traces it (robigo-heap's freeze.rs,
    // docs/ledger/g3/2026-11-21-h-boot-freeze-check.md); off under
    // ROBIGO_GC_MOVE's modes
    cx.freeze_boot_heap();
    // the isolate's console is the process's stdout and stderr
    // E9's ROBIGO_FAIL_ALLOC_AT counts the script's allocations, not base's
    cx.arm_fail_alloc();
    install_sigint();
    // end_Rmainloop's R_CleanUp(SA_DEFAULT, 0, 1) runs .Last; after an
    // error, check_session_exit's R_CleanUp(SA_NOSAVE, 1, 0) does not
    // (end_Rmainloop's newline unless R_NoEcho, then .Last: run_session)
    let ok = if stdin {
        run_session(cx, &mut robigo_rt::repl::Input::Stdin(robigo_rt::repl::StdinBuf::new()))
    } else {
        run_session(cx, &mut robigo_rt::repl::Input::Bytes(&src))
    };
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
            missing_tree(&RHome::MissingDefault(h.clone()), Boot::Stage0).as_deref(),
            Some(
                "R_HOME ('/nonexistent/robigo/.robigo/rt-home') not found: no installed tree \
                 (<prefix>/lib/robigo) has library/base/R/base, and the checkout's is not made (scripts/rt-home.sh)"
            )
        );
        assert!(
            missing_tree(&RHome::MissingDefault(h.clone()), Boot::Rdb).unwrap().ends_with("(scripts/rt-home-db.sh)")
        );
        assert_eq!(missing_tree(&RHome::Given(h), Boot::Rdb), None);
    }
}
