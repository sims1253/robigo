use std::io::{BufRead, Write};
use std::process::ExitCode;

use robigo_runtime::Interp;

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
        let p = unsafe { if layout.align() <= WORD { mi_malloc(layout.size()) } else { mi_malloc_aligned(layout.size(), layout.align()) } };
        p as *mut u8
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        use libmimalloc_sys::*;
        // SAFETY: as in `alloc`.
        let p = unsafe { if layout.align() <= WORD { mi_zalloc(layout.size()) } else { mi_zalloc_aligned(layout.size(), layout.align()) } };
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
        let q = unsafe { if layout.align() <= WORD { mi_realloc(p, new_size) } else { mi_realloc_aligned(p, new_size, layout.align()) } };
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
    let mut load_base = true;
    let mut startup = robigo_runtime::startup::StartupOptions::default();
    let mut exprs: Vec<String> = Vec::new();
    let mut file: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-base" => load_base = false,
            "--vanilla" => {
                startup.no_environ = true;
                startup.no_site_file = true;
                startup.no_init_file = true;
            }
            "--no-environ" => startup.no_environ = true,
            "--no-site-file" => startup.no_site_file = true,
            "--no-init-file" => startup.no_init_file = true,
            "--no-echo" | "--quiet" | "-q" | "--slave" | "--no-restore" => {}
            "-e" => {
                i += 1;
                match args.get(i) {
                    Some(e) => exprs.push(e.clone()),
                    None => {
                        eprintln!("{}", USAGE);
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{}", USAGE);
                return ExitCode::SUCCESS;
            }
            _ if file.is_none() => {
                file = Some(args[i].clone());
                // Remaining args belong to the script (commandArgs()).
                args.truncate(i + 1);
                break;
            }
            _ => {}
        }
        i += 1;
    }

    #[cfg(feature = "profile")]
    let _profiler = Profiler::start();
    let mut interp = Interp::new();
    // Keep a margin for builtins that recurse in Rust and for error handling.
    interp.set_stack_limit(STACK_SIZE - (32 << 20));
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
        if !robigo_runtime::startup::run(&mut interp, startup) {
            return finish(&mut interp, false);
        }
    }

    if !exprs.is_empty() {
        let src = exprs.join("\n");
        let ok = interp.run_source(&src, true);
        return finish(&mut interp, ok);
    }
    if let Some(path) = file {
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("robigo: cannot open file '{}': {}", path, e);
                return ExitCode::from(2);
            }
        };
        interp.source_name = path.clone();
        let ok = interp.run_source(&src, true);
        if robigo_runtime::typeprof::enabled() {
            let g = interp.global.clone();
            robigo_runtime::typeprof::record_frame(None, &g);
            robigo_runtime::typeprof::write();
        }
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
