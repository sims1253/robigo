use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::process::ExitCode;

use robigo_runtime::Interp;

mod echo;

/// mimalloc, calling its plain entry points for word-aligned layouts (all
/// mimalloc blocks are word-aligned); the `mimalloc` crate always takes the
/// slower `*_aligned` path.
struct Allocator;

const WORD: usize = std::mem::size_of::<usize>();

unsafe impl std::alloc::GlobalAlloc for Allocator {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        use libmimalloc_sys::*;
        // SAFETY: plain calls into mimalloc with the requested layout.
        let p = unsafe {
            if layout.align() <= WORD {
                mi_malloc(layout.size())
            } else {
                mi_malloc_aligned(layout.size(), layout.align())
            }
        };
        p as *mut u8
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        use libmimalloc_sys::*;
        // SAFETY: as in `alloc`.
        let p = unsafe {
            if layout.align() <= WORD {
                mi_zalloc(layout.size())
            } else {
                mi_zalloc_aligned(layout.size(), layout.align())
            }
        };
        p as *mut u8
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: std::alloc::Layout) {
        // SAFETY: `ptr` was allocated by mimalloc.
        unsafe { libmimalloc_sys::mi_free(ptr as *mut std::ffi::c_void) };
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        use libmimalloc_sys::*;
        let p = ptr as *mut std::ffi::c_void;
        // SAFETY: `ptr` was allocated by mimalloc with `layout`.
        let q = unsafe {
            if layout.align() <= WORD {
                mi_realloc(p, new_size)
            } else {
                mi_realloc_aligned(p, new_size, layout.align())
            }
        };
        q as *mut u8
    }
}

#[global_allocator]
static GLOBAL: Allocator = Allocator;

const USAGE: &str = "\
usage: robigo [options] [FILE]

  robigo FILE            run an R script (like Rscript FILE)
  robigo -e EXPR         evaluate an expression
  robigo                 start an interactive session
  robigo parse FILE      print the parse tree in the oracle dump format

options:
  --no-base          do not load the base R library
  --vanilla          accepted for compatibility with Rscript
";

/// Rust stack of the interpreter thread. R code may nest calls up to
/// `expressions` (5000) deep, which needs more than a default 8 MB main
/// thread; only the pages actually used are committed.
const STACK_SIZE: usize = 512 << 20;

fn main() -> ExitCode {
    // compiled code goes through libR's API in this executable (robigo-capi)
    R::bridge::install();
    // relink when the exported API changes (build.rs)
    std::hint::black_box(env!("ROBIGO_EXPORTS_HASH"));
    let thread = std::thread::Builder::new().name("robigo".into()).stack_size(STACK_SIZE).spawn(real_main);
    match thread.map(|t| t.join()) {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => ExitCode::from(101), // panicked; the message is printed already
        Err(e) => {
            eprintln!("robigo: cannot start the interpreter thread: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// R's save actions (`SA_TYPE`, R: src/include/R_ext/RStartup.h).
#[derive(PartialEq, Clone, Copy)]
enum SaveAction {
    Save,
    NoSave,
    Ask,
}

/// The command line the way R's option processing leaves it
/// (R: src/main/CommandLineArgs.c:R_common_command_line followed by the
/// argument loop of src/unix/system.c:Rf_initialize_R).
struct CommandLine {
    startup: robigo_runtime::startup::StartupOptions,
    quiet: bool,   // Rp->R_Quiet
    no_echo: bool, // Rp->R_NoEcho
    save: SaveAction,
    verbose: bool,
    force_interactive: bool,
    exprs: Vec<String>,   // -e EXPR, in order
    file: Option<String>, // -f FILE / --file=FILE ('-' is stdin)
    /// robigo's `Rscript FILE` mode: the positional file name. R itself
    /// would print `ARGUMENT ... __ignored__` and read the console
    /// instead; `Rscript` execs `R --no-echo --no-restore --file=FILE`.
    script: Option<String>,
    script_args: Vec<String>,
    /// What `commandArgs()` returns (R: `R_set_command_line_arguments`
    /// stores the argv before option processing).
    command_args: Vec<String>,
}

impl Default for CommandLine {
    fn default() -> Self {
        CommandLine {
            startup: robigo_runtime::startup::StartupOptions::default(),
            quiet: false,
            no_echo: false,
            save: SaveAction::Ask,
            verbose: false,
            force_interactive: false,
            exprs: Vec::new(),
            file: None,
            script: None,
            script_args: Vec::new(),
            command_args: Vec::new(),
        }
    }
}

/// `R_ShowMessage` during startup (R: unix/sys-std.c:Rstd_ShowMessage):
/// the message and a newline. The messages R builds here usually end in
/// a newline themselves, so a blank line follows; the console is not set
/// up yet, and `Rstd_WriteConsoleEx` writes to stdout.
fn show_message(msg: &str) {
    println!("{}", msg);
}

/// R's `R_Suicide` during option processing (R: unix/sys-std.c:Rstd_Suicide
/// before `R_Consolefile` is set, so the message goes to stdout).
fn suicide(msg: &str) -> ! {
    println!("Fatal error: {}", msg);
    std::process::exit(2);
}

/// The first pass, `R_common_command_line` (R: CommandLineArgs.c):
/// consume the options R shares with every front end, keep the rest for
/// [`initialize_r_arguments`]. `kept` receives the arguments that
/// survive, in order.
fn common_command_line(cl: &mut CommandLine, args: &[String], kept: &mut Vec<String>) {
    let mut processing = true;
    for a in args {
        if processing && a.starts_with('-') && a.len() > 1 {
            match a.as_str() {
                "--save" => cl.save = SaveAction::Save,
                "--no-save" => cl.save = SaveAction::NoSave,
                "--restore" | "--no-restore" | "--no-restore-data" | "--no-restore-history" => {}
                "--silent" | "--quiet" | "-q" => cl.quiet = true,
                "--vanilla" => {
                    cl.save = SaveAction::NoSave;
                    cl.startup.no_environ = true;
                    cl.startup.no_site_file = true;
                    cl.startup.no_init_file = true;
                }
                "--no-environ" => cl.startup.no_environ = true,
                "--no-site-file" => cl.startup.no_site_file = true,
                "--no-init-file" => cl.startup.no_init_file = true,
                "--verbose" => cl.verbose = true,
                "--no-echo" | "--slave" | "-s" => {
                    cl.quiet = true;
                    cl.no_echo = true;
                    cl.save = SaveAction::NoSave;
                }
                "--args" => {
                    kept.push(a.clone());
                    processing = false;
                }
                "-save" | "-nosave" | "-restore" | "-norestore" | "-noreadline" | "-quiet" | "-nsize" | "-vsize"
                | "-V" | "-n" | "-v" => {
                    show_message(&format!("WARNING: option '{}' no longer supported", a));
                }
                _ if a.starts_with("--min-nsize")
                    || a.starts_with("--min-vsize")
                    || a.starts_with("--max-ppsize")
                    || a.starts_with("--max-connections")
                    || a.starts_with("--encoding") =>
                {
                    // Options with a value; accepted and ignored (robigo
                    // has no nsize/vsize limits and one input encoding).
                }
                _ => kept.push(a.clone()), // unknown -option: warned about next
            }
        } else {
            kept.push(a.clone());
        }
    }
}

/// The second pass, the argument loop of `Rf_initialize_R` (R:
/// unix/system.c): `-f`, `--file=`, `-e` and `--args`, and R's warnings
/// for everything it does not know. robigo's `Rscript FILE` mode takes
/// the first remaining positional argument as the script to run.
fn initialize_r_arguments(cl: &mut CommandLine, kept: Vec<String>) {
    let mut i = 0;
    while i < kept.len() {
        let a = kept[i].clone();
        if a.starts_with('-') && a.len() > 1 {
            match a.as_str() {
                "--no-readline" => {}
                "-f" => {
                    i += 1;
                    match kept.get(i) {
                        Some(f) => cl.file = Some(unescape_arg(f)),
                        None => {
                            // The message of the R shell wrapper, which
                            // validates -f before exec'ing R (R:
                            // scripts/R.sh.in).
                            eprintln!("ERROR: option '-f' requires a filename argument");
                            std::process::exit(1);
                        }
                    }
                }
                _ if a.starts_with("--file=") => cl.file = Some(unescape_arg(&a[7..])),
                "-e" => {
                    i += 1;
                    match kept.get(i) {
                        Some(e) => {
                            cl.exprs.push(unescape_arg(e));
                            // robigo's `-e` is `Rscript -e`: Rscript
                            // execs R with `--no-echo` (R: unix/Rscript.c),
                            // which also suppresses prompts and echo.
                            cl.no_echo = true;
                        }
                        None => {
                            eprintln!("ERROR: option '-e' requires a non-empty argument");
                            std::process::exit(1);
                        }
                    }
                }
                "--args" => break,
                "--interactive" => {
                    cl.force_interactive = true;
                    break;
                }
                _ => show_message(&format!("WARNING: unknown option '{}'\n", a)),
            }
        } else if cl.file.is_none() && cl.exprs.is_empty() && cl.script.is_none() {
            // robigo's Rscript mode: `robigo [options] FILE [args]`, the
            // way Rscript runs `R --no-echo --no-restore --file=FILE`.
            cl.script = Some(a);
            cl.script_args = kept[i + 1..].to_vec();
            cl.no_echo = true;
            return;
        } else {
            show_message(&format!("ARGUMENT '{}' __ignored__\n", a));
        }
        i += 1;
    }
}

/// `robigo CMD command args`: GNU R's `R CMD` (its `bin/R` script runs
/// `bin/Rcmd`, whose tools run `bin/R` again) in robigo's home, where
/// that `R` is robigo (`scripts/robigo-home.sh`, docs/core.md §8).
fn r_cmd(args: &[String]) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let is_ours = |home: &str| {
        let r = std::path::Path::new(home).join("bin/exec/R");
        r.canonicalize().ok().is_some_and(|r| Some(r) == me)
    };
    let home = robigo_runtime::startup::robigo_home().or_else(|| std::env::var("R_HOME").ok().filter(|h| is_ours(h)));
    let Some(home) = home else {
        eprintln!("robigo: 'CMD' needs robigo's R_HOME (scripts/robigo-home.sh)");
        return ExitCode::FAILURE;
    };
    let err = std::process::Command::new(format!("{home}/bin/R")).arg("CMD").args(args).exec();
    eprintln!("robigo: cannot run {home}/bin/R: {err}");
    ExitCode::FAILURE
}

/// Undo the escaping of GNU R's front-end script (`bin/R` passes `-e`
/// expressions and `-f` paths with `~+~` for spaces, `~n~` for newlines,
/// `~t~` for tabs); R's binary undoes it for every caller, Rscript's too.
// R: unix/system.c:unescape_arg
fn unescape_arg(a: &str) -> String {
    let mut out = String::with_capacity(a.len());
    let mut rest = a;
    while let Some(c) = rest.chars().next() {
        let r = rest.as_bytes();
        let sub = match (c, r.get(1), r.get(2)) {
            ('~', Some(b'+'), Some(b'~')) => Some(' '),
            ('~', Some(b'n'), Some(b'~')) => Some('\n'),
            ('~', Some(b't'), Some(b'~')) => Some('\t'),
            _ => None,
        };
        match sub {
            Some(x) => {
                out.push(x);
                rest = &rest[3..];
            }
            None => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

/// What `commandArgs()` reports: the arguments of the R process, the way
/// the front end passed them (R: CommandLineArgs.c:do_commandArgs sees
/// the `argv` `R_set_command_line_arguments` copied). `Rscript` execs
/// `R --no-echo --no-restore [options] --file=FILE [--args ...]`
/// (R: unix/Rscript.c), which is what robigo's positional file stands
/// for; with `-f` robigo is `R` itself and the argv passes through.
fn command_args(argv0: &str, args: &[String], cl: &CommandLine) -> Vec<String> {
    let mut out = vec![argv0.to_string()];
    // Rscript mode (a positional file, or `-e`): the argv Rscript would
    // have exec'd `R` with (R: unix/Rscript.c). `i0` is the index (in
    // `args`) of the last argument Rscript copies; the file name is the
    // next one, and any arguments after it follow `--args`.
    if cl.script.is_some() || !cl.exprs.is_empty() {
        out.push("--no-echo".into());
        out.push("--no-restore".into());
        let mut e_mode = false;
        let mut i0: isize = -1;
        let mut k = 0usize;
        while k < args.len() {
            if args[k] == "-e" {
                e_mode = true;
                out.push("-e".into());
                if let Some(e) = args.get(k + 1) {
                    out.push(e.clone());
                }
                i0 = k as isize + 1;
                k += 2;
                continue;
            }
            if e_mode {
                // Once in e_mode, only additional -e options are to be
                // processed; any remaining --options belong to the
                // expressions (PR#18102).
                break;
            }
            if !args[k].starts_with("--") {
                break; // the file
            }
            if args[k] == "--verbose" || args[k].starts_with("--default-packages=") {
                // Consumed by Rscript itself, not passed to R.
                i0 = k as isize;
                k += 1;
                continue;
            }
            out.push(args[k].clone());
            i0 = k as isize;
            k += 1;
        }
        if !e_mode && i0 + 1 < args.len() as isize {
            out.push(format!("--file={}", args[(i0 + 1) as usize]));
        }
        if i0 + 2 < args.len() as isize {
            out.push("--args".into());
            out.extend(args[(i0 + 2) as usize..].iter().cloned());
        }
    } else {
        // `-f`/`--file=` and the console: `R`'s own argv.
        out.extend(args.iter().cloned());
    }
    out
}

/// Was robigo invoked Rscript-style — a positional file, or `-e`
/// expressions — rather than as `R` (`-f`/`--file=`, or the console)?
/// The distinction decides who consumes `--default-packages=` (R: unix/
/// Rscript.c) and how `commandArgs()` is reported.
fn is_rscript_invocation(args: &[String]) -> bool {
    let mut e_mode = false;
    let mut k = 0;
    while k < args.len() {
        if args[k] == "-e" {
            e_mode = true;
            k += 2;
            continue;
        }
        if e_mode {
            break;
        }
        match args[k].as_str() {
            "-" => return true, // the script is stdin
            "-f" | "--args" | "--interactive" => return false,
            a if a.starts_with("--file=") => return false,
            // Only `--` options are forwarded to R; another single-dash
            // option (`-q`, ...) makes this an `R` invocation.
            a if !a.starts_with('-') => return true, // the file
            _ => {}
        }
        k += 1;
    }
    e_mode
}

fn real_main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("parse") {
        return parse_cmd(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("CMD") {
        return r_cmd(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("--list-builtins") {
        for b in robigo_runtime::builtins::all_builtins() {
            println!("{}\t{}", b.name, if b.internal { "internal" } else { "primitive" });
        }
        return ExitCode::SUCCESS;
    }
    // robigo's own help (R has none; the options end at the first
    // positional argument, like Rscript's).
    let opts_end = args.iter().position(|a| !(a.starts_with('-') && a.len() > 1)).unwrap_or(args.len());
    if args[..opts_end].iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", USAGE);
        return ExitCode::SUCCESS;
    }
    let mut load_base = true;
    // robigo's own option, not R's: do not load the base library.
    if let Some(pos) = args.iter().position(|a| a == "--no-base") {
        load_base = false;
        args.remove(pos);
    }
    // In Rscript mode (a positional file, or `-e`), Rscript consumes
    // `--default-packages=` itself and exports the value (R:
    // unix/Rscript.c). As R's own option (with `-f`/`--file=`, at the
    // console) it stays unknown and is warned about below, exactly like
    // `R` does.
    if is_rscript_invocation(&args) {
        let mut k = 0;
        let mut e_mode = false;
        while k < args.len() {
            if args[k] == "-e" {
                e_mode = true;
                k += 2;
                continue;
            }
            if e_mode || !args[k].starts_with("--") {
                break; // the script (its arguments follow it)
            }
            if let Some(v) = args[k].strip_prefix("--default-packages=") {
                // SAFETY: single-threaded at this point.
                unsafe { std::env::set_var("R_DEFAULT_PACKAGES", v) };
                args.remove(k);
                continue;
            }
            k += 1;
        }
    }
    let mut cl = CommandLine::default();
    let mut kept = Vec::new();
    common_command_line(&mut cl, &args, &mut kept);
    initialize_r_arguments(&mut cl, kept);
    let argv0 = std::env::args().next().unwrap_or_else(|| "robigo".into());
    cl.command_args = command_args(&argv0, &args, &cl);
    if !cl.exprs.is_empty() && cl.file.is_some() {
        suicide("cannot use -e with -f or --file");
    }

    #[cfg(feature = "profile")]
    let _profiler = Profiler::start();
    // R_Interactive is known before base/profile/`.First` startup code runs
    // (R: system.c's R_isatty(0) test), and `interactive()`/`q()` in those
    // hooks must see it.
    let tty = std::io::stdin().is_terminal();
    let has_console_input = !cl.exprs.is_empty() || cl.script.is_some() || cl.file.is_some();
    let mut interp = Interp::new();
    interp.interactive = !has_console_input && (cl.force_interactive || tty);
    // Keep a margin for builtins that recurse in Rust and for error handling.
    interp.set_stack_limit(STACK_SIZE - (32 << 20));
    interp.cli_args = Some(cl.command_args.clone());
    if load_base {
        if let Err(msg) = robigo_runtime::bootstrap::load_base(&mut interp) {
            eprintln!("robigo: failed to load base library: {}", msg);
            return ExitCode::FAILURE;
        }
        if !robigo_runtime::startup::run(&mut interp, cl.startup) {
            return finish(&mut interp, false);
        }
    }

    // What feeds the console: the expressions of `-e` (a temporary file
    // for R), the file of `-f`/`--file=`/a positional script, or stdin.
    let script_path = cl.script.clone();
    let console_input = if !cl.exprs.is_empty() {
        let mut cmdlines = cl.exprs.join("\n");
        cmdlines.push('\n');
        Some((Box::new(std::io::Cursor::new(cmdlines)) as Box<dyn Read>, "<text>".to_string()))
    } else if let Some(path) = script_path.or(cl.file.clone()) {
        let read: Box<dyn Read> = if path == "-" {
            Box::new(BufReader::new(std::io::stdin()))
        } else {
            match std::fs::File::open(&path) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    let reason = e.to_string();
                    let reason = reason.split(" (os error").next().unwrap_or("").to_string();
                    suicide(&format!("cannot open file '{}': {}", path, reason));
                }
            }
        };
        Some((read, path))
    } else {
        None
    };
    if let Some((read, name)) = console_input {
        // R: system.c — a file (or `-e`) console implies `--no-save`
        // unless `--save` was given.
        if cl.save != SaveAction::Save {
            cl.save = SaveAction::NoSave;
        }
        interp.source_name = name.clone();
        let mut input = echo::ConsoleInput::new(read);
        let echo = !cl.no_echo;
        let ok = echo::repl_console(&mut interp, &mut input, echo);
        if robigo_runtime::typeprof::enabled() {
            let g = interp.global.clone();
            robigo_runtime::typeprof::record_frame(None, &g);
            robigo_runtime::typeprof::write();
        }
        return finish(&mut interp, ok);
    }
    // No file and no `-e`: interactive when the terminal is one or
    // `--interactive` was given, otherwise stdin is read as a
    // non-interactive console (R: system.c's `R_isatty(0)` test).
    if !cl.force_interactive && !tty {
        if cl.save != SaveAction::Save && cl.save != SaveAction::NoSave {
            // R: system.c — `R_Suicide` once the console is set up, so
            // this one goes to stderr.
            eprintln!("Fatal error: you must specify '--save', '--no-save' or '--vanilla'");
            std::process::exit(2);
        }
        let mut input = echo::ConsoleInput::new(Box::new(BufReader::new(std::io::stdin())));
        let ok = echo::repl_console(&mut interp, &mut input, !cl.no_echo);
        return finish(&mut interp, ok);
    }
    repl(&mut interp)
}

/// With `--features profile`, `ROBIGO_PROFILE=out.svg robigo script.R` writes a
/// flamegraph of the run.
#[cfg(feature = "profile")]
struct Profiler(Option<pprof::ProfilerGuard<'static>>);

#[cfg(feature = "profile")]
impl Profiler {
    fn start() -> Profiler {
        if std::env::var_os("ROBIGO_PROFILE").is_none() {
            return Profiler(None);
        }
        Profiler(pprof::ProfilerGuardBuilder::default().frequency(997).build().ok())
    }
}

#[cfg(feature = "profile")]
impl Drop for Profiler {
    fn drop(&mut self) {
        if let (Some(g), Some(path)) = (self.0.take(), std::env::var_os("ROBIGO_PROFILE")) {
            if let Ok(report) = g.report().build() {
                if path.to_string_lossy().ends_with(".txt") {
                    let _ = std::fs::write(&path, line_profile(&report));
                } else if let Ok(f) = std::fs::File::create(&path) {
                    let _ = report.flamegraph(f);
                }
            }
        }
    }
}

/// Self time by source line: for the innermost physical frame of each
/// sample, the innermost inlined function's line and the line of the
/// physical function it was inlined into.
#[cfg(feature = "profile")]
fn line_profile(report: &pprof::Report) -> String {
    use std::collections::HashMap;
    let mut inner: HashMap<String, isize> = HashMap::new();
    let mut outer: HashMap<String, isize> = HashMap::new();
    let mut total = 0;
    let loc = |s: &pprof::Symbol| {
        let file = s.filename.as_ref().and_then(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned());
        format!("{} {}:{}", s.name(), file.unwrap_or_default(), s.lineno.unwrap_or(0))
    };
    for (frames, n) in &report.data {
        total += n;
        let Some(f0) = frames.frames.first() else { continue };
        if let Some(s) = f0.first() {
            *inner.entry(loc(s)).or_default() += n;
        }
        if let Some(s) = f0.last() {
            *outer.entry(loc(s)).or_default() += n;
        }
    }
    let mut out = format!("{} samples\n", total);
    for (title, map) in [("innermost (inlined) location", inner), ("location in the physical function", outer)] {
        out.push_str(&format!("\n== {}\n", title));
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in v.into_iter().take(40) {
            out.push_str(&format!("{:5.1}%  {}\n", 100.0 * n as f64 / total as f64, k));
        }
    }
    out
}

fn finish(interp: &mut Interp, ok: bool) -> ExitCode {
    // R: unix/sys-std.c:Rstd_CleanUp -- .Last runs on a normal exit, then
    // R_RunExitFinalizers runs on every exit path. The top-level handler
    // prints "Execution halted" before the finalizers, including when
    // .Last itself fails.
    let mut ok = ok;
    if ok {
        if let Err(f) = interp.run_dot_last() {
            interp.report_toplevel(f);
            ok = false;
        }
    }
    if !ok {
        interp.write_err("Execution halted\n");
    }
    cleanup(interp);
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// The tail of `Rstd_CleanUp`: exit finalizers, then warnings raised by
/// them (or by `.Last`).
fn cleanup(interp: &mut Interp) {
    if let Some(bridge) = robigo_runtime::native::bridge() {
        bridge.run_exit_finalizers(interp);
    }
    // R: unix/sys-std.c:Rstd_CleanUp -- PrintWarnings after the finalizers
    // (device close and, if run, .Last).
    interp.print_warnings();
    interp.flush();
}

/// Interactive EOF. R: main.c:R_dot_Last -- an error in `.Last` kicks the
/// session back to the REPL, before the exit finalizers; otherwise the
/// session leaves. `None` means the REPL keeps running.
fn finish_interactive(interp: &mut Interp) -> Option<ExitCode> {
    if let Err(f) = interp.run_dot_last() {
        interp.report_toplevel(f);
        return None;
    }
    cleanup(interp);
    Some(ExitCode::SUCCESS)
}

fn repl(interp: &mut Interp) -> ExitCode {
    let stdin = std::io::stdin();
    let mut buf = String::new();
    loop {
        print!("{}", if buf.is_empty() { "> " } else { "+ " });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                match finish_interactive(interp) {
                    Some(code) => return code,
                    // .Last failed: it was reported, and R is back at the
                    // prompt with the quit cancelled.
                    None => continue,
                }
            }
            Ok(_) => {}
            Err(_) => return ExitCode::FAILURE,
        }
        buf.push_str(&line);
        let arena = robigo_syntax::Arena::new();
        match robigo_syntax::parse_program(&arena, &buf) {
            Err(e) if e.incomplete => continue,
            _ => {
                let src = std::mem::take(&mut buf);
                interp.run_source(&src, true);
                interp.flush();
            }
        }
    }
}

fn parse_cmd(args: &[String]) -> ExitCode {
    let Some(path) = args.first() else {
        eprint!("{}", USAGE);
        return ExitCode::from(2);
    };
    let src = String::from_utf8_lossy(&std::fs::read(path).expect("read file")).into_owned();
    let arena = robigo_syntax::Arena::new();
    match robigo_syntax::parse_program(&arena, &src) {
        Ok(exprs) => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            for e in &exprs {
                out.write_all(&robigo_syntax::dump::dump(e)).unwrap();
                out.write_all(b"\n").unwrap();
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let (line, col) = robigo_syntax::error_line_col(&src, &e);
            eprintln!("{}:{}:{}: {}", path, line, col, e.message);
            ExitCode::FAILURE
        }
    }
}
