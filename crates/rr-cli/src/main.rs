use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: rr parse FILE");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("parse") => {
            let Some(path) = args.get(1) else { return usage() };
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
        _ => usage(),
    }
}
