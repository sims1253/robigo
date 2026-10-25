use std::io::{BufRead, Write};
use std::process::ExitCode;

use rr_runtime::Interp;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "\
usage: rr [options] [FILE]

  rr FILE            run an R script (like Rscript FILE)
  rr -e EXPR         evaluate an expression
  rr                 start an interactive session
  rr parse FILE      print the parse tree in the oracle dump format

options:
  --no-base          do not load the base R library
  --vanilla          accepted for compatibility with Rscript
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("parse") {
        return parse_cmd(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("--list-builtins") {
        for b in rr_runtime::builtins::all_builtins() {
            println!("{}\t{}", b.name, if b.internal { "internal" } else { "primitive" });
        }
        return ExitCode::SUCCESS;
    }
    let mut load_base = true;
    let mut exprs: Vec<String> = Vec::new();
    let mut file: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-base" => load_base = false,
            "--vanilla" | "--no-echo" | "--quiet" | "-q" | "--slave" => {}
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
    if load_base {
        if let Err(msg) = rr_runtime::bootstrap::load_base(&mut interp) {
            eprintln!("rr: failed to load base library: {}", msg);
            return ExitCode::FAILURE;
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
                eprintln!("rr: cannot open file '{}': {}", path, e);
                return ExitCode::from(2);
            }
        };
        let ok = interp.run_source(&src, true);
        return finish(&mut interp, ok);
    }
    repl(&mut interp)
}

/// With `--features profile`, `RR_PROFILE=out.svg rr script.R` writes a
/// flamegraph of the run.
#[cfg(feature = "profile")]
struct Profiler(Option<pprof::ProfilerGuard<'static>>);

#[cfg(feature = "profile")]
impl Profiler {
    fn start() -> Profiler {
        if std::env::var_os("RR_PROFILE").is_none() {
            return Profiler(None);
        }
        Profiler(pprof::ProfilerGuardBuilder::default().frequency(997).build().ok())
    }
}

#[cfg(feature = "profile")]
impl Drop for Profiler {
    fn drop(&mut self) {
        if let (Some(g), Some(path)) = (self.0.take(), std::env::var_os("RR_PROFILE")) {
            if let Ok(report) = g.report().build() {
                if let Ok(f) = std::fs::File::create(&path) {
                    let _ = report.flamegraph(f);
                }
            }
        }
    }
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
        match rr_syntax::parse_program(&buf) {
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
    match rr_syntax::parse_program(&src) {
        Ok(exprs) => {
            for e in &exprs {
                println!("{}", rr_syntax::dump::dump(e));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let (line, col) = rr_syntax::line_col(&src, e.offset);
            eprintln!("{}:{}:{}: {}", path, line, col, e.message);
            ExitCode::FAILURE
        }
    }
}
