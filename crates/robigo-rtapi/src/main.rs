//! `robigo-rt [--vanilla] FILE | -e EXPR ...`: Rscript's top level over G3's
//! runtime (`docs/g3.md` §14.2, M1): the isolate made, the evaluator and the
//! C API installed, the base subset bound from the pinned GNU R sources
//! (`robigo_rt::base`), and the script run as Rscript runs it
//! (`robigo_rt::repl`). `R_DEFAULT_PACKAGES` is ignored: there are no other
//! packages. Exit status 0, or 1 after "Execution halted"; 2 for a usage or
//! start-up failure.

use robigo_rt::repl::run_console;
use robigo_rtcore::OwnedIsolate;

const USAGE: &str = "Usage: robigo-rt [--vanilla] [options] file [args]\n   or: robigo-rt [--vanilla] [options] -e expr [-e expr2 ...] [args]\n";

fn main() {
    std::process::exit(run(std::env::args_os().skip(1).collect()));
}

fn run(args: Vec<std::ffi::OsString>) -> i32 {
    // Rscript's options: R's start-up options, then FILE or -e EXPR, then
    // the script's arguments (commandArgs(), not yet read by anything)
    let mut exprs: Vec<Vec<u8>> = Vec::new();
    let mut file = None;
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
            b"--help" | b"-h" => {
                print!("{USAGE}");
                return 0;
            }
            // start-up options that change nothing robigo-rt has
            b"--vanilla" | b"--no-echo" | b"--quiet" | b"--silent" | b"--slave" | b"--no-save" | b"--no-restore"
            | b"--no-environ" | b"--no-site-file" | b"--no-init-file" | b"--verbose" => {}
            _ if b.starts_with(b"--default-packages=") => {}
            _ if b.starts_with(b"-") && exprs.is_empty() && file.is_none() => {
                eprintln!("robigo-rt: unknown option '{}'", a.to_string_lossy());
                return 2;
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
    let src = if !exprs.is_empty() {
        // R -e: the expressions, one per line
        let mut s = Vec::new();
        for e in exprs {
            s.extend_from_slice(&e);
            s.push(b'\n');
        }
        s
    } else if let Some(f) = file {
        match std::fs::read(&f) {
            Ok(s) => s,
            Err(e) => {
                // Rscript: R's "cannot open file" error, then halts
                eprintln!("Fatal error: cannot open file '{}': {}", f.to_string_lossy(), io_reason(&e));
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
    robigo_rt::install(cx);
    // `.Call` and `dyn.load`, before base binds its stubs
    robigo_rtapi::install(cx);
    if let Err(e) = robigo_rt::base::load(cx, &dir) {
        eprintln!("robigo-rt: loading base: {e}");
        return 2;
    }
    // the isolate's console is the process's stdout and stderr
    let ok = run_console(cx, &src);
    if !ok {
        // R: main.c:check_session_exit
        cx.reprintf(b"Execution halted\n");
    }
    // R_CleanUp: (no .Last yet) the exit finalizers, then exit
    // R: unix/sys-std.c:Rstd_CleanUp
    robigo_rt::fin::run_exit_finalizers(cx);
    cx.flush_console();
    if ok { 0 } else { 1 }
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
