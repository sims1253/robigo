//! Loading the base library: GNU R's base-package R sources evaluated in
//! the base environment on top of robigo's builtins.

use std::io::Write;

use crate::interp::Interp;
use crate::lang::expr_to_value;

static BASE_SOURCES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/base_sources.rs"));

/// One top-level base expression that failed to evaluate.
pub struct BootFailure {
    pub file: &'static str,
    pub line: usize,
    pub message: String,
}

/// Evaluate all base sources. Failures of individual top-level
/// expressions are collected rather than aborting, so that the rest of
/// base still loads while the runtime is incomplete.
pub fn load_base_report(interp: &mut Interp) -> Result<Vec<BootFailure>, String> {
    let mut failures = Vec::new();
    let base = interp.base_ns.clone();
    // Errors during bootstrap must not be printed.
    let saved_err = std::mem::replace(&mut interp.err, Box::new(std::io::sink()));
    for (file, src) in BASE_SOURCES {
        // The configure substitution R's build applies (basepkg.mk).
        let configured;
        let src: &str = if src.contains("@WHICH@") {
            configured = src.replace("@WHICH@", &find_program("which"));
            &configured
        } else {
            src
        };
        // The arena owns the trees; both die at the end of this block,
        // after the conversion to values.
        let arena = robigo_syntax::Arena::new();
        let exprs = robigo_syntax::parse_program(&arena, src).map_err(|e| {
            let (l, c) = robigo_syntax::line_col(src, e.offset);
            format!("{}:{}:{}: {}", file, l, c, e.message)
        })?;
        for e in &exprs {
            let v = expr_to_value(e);
            interp.frames.clear();
            interp.handlers = None;
            interp.restarts = None;
            match interp.eval(&v, &base) {
                Ok(_) => {}
                Err(f) => {
                    let message = match f {
                        crate::error::Flow::Error(e) => e.message,
                        _ => "unexpected control flow".to_string(),
                    };
                    let (line, _) = robigo_syntax::line_col(src, e.span().start as usize);
                    failures.push(BootFailure { file, line, message });
                }
            }
        }
    }
    // What R's baseloader.R does after loading base: bind the base DLL's
    // registered routines as .F_xxx and .C_xxx.
    let tail = r#"local({
    routines <- getDLLRegisteredRoutines("base")
    for (i in c("dchdc", "dqrcf", "dqrdc2", "dqrqty", "dqrqy", "dqrrsd", "dqrxb", "dtrco"))
        assign(paste0(".F_", i), routines[[3]][[i]], envir = .BaseNamespaceEnv)
    for(i in 1:2)
        lapply(routines[[i]],
               function(sym) assign(paste0(".C_", sym$name), sym, envir = .BaseNamespaceEnv))
})"#;
    let tail_arena = robigo_syntax::Arena::new();
    for e in robigo_syntax::parse_program(&tail_arena, tail).map_err(|e| e.message)? {
        interp.frames.clear();
        if let Err(crate::error::Flow::Error(e)) = interp.eval(&expr_to_value(&e), &base) {
            failures.push(BootFailure { file: "baseloader", line: 0, message: e.message });
        }
    }
    interp.warnings.clear();
    interp.err = saved_err;
    crate::builtins::post_base_init(interp);
    Ok(failures)
}

/// The absolute path of `prog` on `PATH`, or "" (configure's
/// `AC_PATH_PROGS`).
fn find_program(prog: &str) -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|d| d.join(prog))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

pub fn load_base(interp: &mut Interp) -> Result<(), String> {
    let failures = load_base_report(interp)?;
    if std::env::var_os("ROBIGO_BOOT_VERBOSE").is_some() {
        let mut err = std::io::stderr();
        for f in &failures {
            let _ = writeln!(err, "boot: {}:{}: {}", f.file, f.line, f.message);
        }
        let _ = writeln!(err, "boot: {} failing top-level expressions", failures.len());
    }
    Ok(())
}
