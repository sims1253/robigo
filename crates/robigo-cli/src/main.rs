use std::io::{BufRead, Write};
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
    quiet: bool,        // Rp->R_Quiet
    no_echo: bool,      // Rp->R_NoEcho
    save: SaveAction,
    verbose: bool,
    force_interactive: bool,
    exprs: Vec<String>,     // -e EXPR, in order
    file: Option<String>,   // -f FILE / --file=FILE ('-' is stdin)
    /// robigo's `Rscript FILE`: the positional file name (R itself would
    /// print `ARGUMENT ... __ignored__` and read the console instead).
    script: Option<String>,
    script_args: Vec<String>,
    /// What `commandArgs()` returns (set once the mode is known).
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

/// `R_ShowMessage` on a file console (R: unix/sys-std.c:Rstd_ShowMessage):
/// the message and a newline, on stderr. The messages themselves end in
/// a newline, so a blank line follows.
fn show_message(msg: &str) {
    eprintln!("{}", msg);
}

/// R's `R_Suicide` (R: unix/sys-std.c:Rstd_Suicide).
fn suicide(msg: &str) -> ! {
    eprintln!("Fatal error: {}", msg);
    std::process::exit(2);
}

/// The first pass, `R_common_command_line`: consume the options R shares
/// with every front end, keep the rest for
/// [`initialize_R_arguments`]. `kept` receives the arguments that
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
                "-save" | "-nosave" | "-restore" | "-norestore" | "-noreadline" | "-quiet"
                | "-nsize" | "-vsize" | "-V" | "-n" | "-v" => {
                    show_message(&format!("WARNING: option '{}' no longer supported\n", a));
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

/// The second pass, the argument loop of `Rf_initialize_R`: `-f`,
/// `--file=`, `-e` and `--args`, and R's warnings for everything it
/// does not know. robigo's own `Rscript FILE` mode takes the first
/// remaining positional argument as the script to run.
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
                        Some(f) => cl.file = Some(f.clone()),
                        None => {
                            eprintln!("ERROR: option '-f' requires a filename argument");
                            std::process::exit(1);
                        }
                    }
                }
                _ if a.starts_with("--file=") => cl.file = Some(a[7..].to_string()),
                "-e" => {
                    i += 1;
                    match kept.get(i) {
                        Some(e) => cl.exprs.push(e.clone()),
                        None => {
                            eprintln!("{}", USAGE);
                            std::process::exit(2);
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

/// What `commandArgs()` reports: the arguments of the R process, the way
/// the front end passed them (R: CommandLineArgs.c:do_commandArgs sees
/// the `argv` `R_set_command_line_arguments` copied). `Rscript` execs
/// `R --no-echo --no-restore [options] --file=FILE [--args ...]`
/// (R: unix/Rscript.c), which is what robigo's positional file stands
/// for; with `-f` robigo is `R` itself and the argv passes through.
fn command_args(argv0: &str, args: &[String], cl: &CommandLine) -> Vec<String> {
    let mut out = vec![argv0.to_string()];
    // Rscript mode (a positional file, or `-e`): the argv Rscript would
    // have exec'd `R` with.
    if cl.script.is_some() || !cl.exprs.is_empty() {
        out.push("--no-echo".into());
        out.push("--no-restore".into());
        let mut k = 0;
        while k < args.len() {
            let a = &args[k];
            match a.as_str() {
                "-e" => {
                    out.push("-e".into());
                    if let Some(e) = args.get(k + 1) {
                        out.push(e.clone());
                    }
                    k += 2;
                    continue;
                }
                a if a.starts_with('-') && a.len() > 1 => out.push(a.to_string()),
                _ => {
                    out.push(format!("--file={}", a));
                    if k + 1 < args.len() {
                        out.push("--args".into());
                        out.extend(args[k + 1..].iter().cloned());
                    }
                    break;
                }
            }
            k += 1;
        }
    } else {
        // `-f`/`--file=` and the console: `R`'s own argv.
        out.extend(args.iter().cloned());
    }
    out
}

fn real_main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("parse") {
        return parse_cmd(&args[1..]);
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
    // robigo's own options, not R's.
    if let Some(pos) = args.iter().position(|a| a == "--no-base") {
        load_base = false;
        args.remove(pos);
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
    let mut interp = Interp::new();
    // Keep a margin for builtins that recurse in Rust and for error handling.
    interp.set_stack_limit(STACK_SIZE - (32 << 20));
    interp.cli_args = Some(cl.command_args.clone());
    if load_base {
        if let Err(msg) = robigo_runtime::bootstrap::load_base(&mut interp) {
            eprintln!("robigo: failed to load base library: {}", msg);
            return ExitCode::FAILURE;
        }
        // robigo cannot load the default packages (utils, stats, ...) yet, so
        // it starts as `R_DEFAULT_PACKAGES=NULL` does unless told otherwise.
        if std::env::var_os("R_DEFAULT_PACKAGES").is_none() {
            // SAFETY: single-threaded at this point.
            unsafe { std::env::set_var("R_DEFAULT_PACKAGES", "NULL") };
        }
        if !robigo_runtime::startup::run(&mut interp, cl.startup) {
            return finish(&mut interp, false);
        }
    }

    // What feeds the console: the expressions of `-e` (a temporary file
    // for R), the file of `-f`/`--file=`, or stdin.
    let script_path = cl.script.clone();
    let console_input = if !cl.exprs.is_empty() {
        let mut cmdlines = cl.exprs.join("\n");
        cmdlines.push('\n');
        Some((Box::new(std::io::Cursor::new(cmdlines)) as Box<dyn std::io::Read>, "<text>".to_string()))
    } else if let Some(path) = script_path.or(cl.file.clone()) {
        let read: Box<dyn std::io::Read> = if path == "-" {
            Box::new(std::io::stdin())
        } else {
            match std::fs::File::open(&path) {
                Ok(f) => Box::new(f),
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
        // unless `--save` was given, and only then is a decision about
        // the workspace required of a non-interactive session.
        if cl.save != SaveAction::Save {
            cl.save = SaveAction::NoSave;
        }
        if cl.save != SaveAction::Save && cl.save != SaveAction::NoSave {
            suicide("you must specify '--save', '--no-save' or '--vanilla'");
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
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if !cl.force_interactive && !tty {
        if cl.save != SaveAction::Save && cl.save != SaveAction::NoSave {
            suicide("you must specify '--save', '--no-save' or '--vanilla'");
        }
        let mut input = echo::ConsoleInput::new(Box::new(std::io::stdin()));
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
    interp.flush();
    if ok {
        ExitCode::SUCCESS
    } else {
        interp.write_err("Execution halted\n");
        interp.flush();
        ExitCode::FAILURE
    }
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
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(_) => return ExitCode::FAILURE,
        }
        buf.push_str(&line);
        match robigo_syntax::parse_program(&buf) {
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
    let src = std::fs::read_to_string(path).expect("read file");
    match robigo_syntax::parse_program(&src) {
        Ok(exprs) => {
            for e in &exprs {
                println!("{}", robigo_syntax::dump::dump(e));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let (line, col) = robigo_syntax::line_col(&src, e.offset);
            eprintln!("{}:{}:{}: {}", path, line, col, e.message);
            ExitCode::FAILURE
        }
    }
}
