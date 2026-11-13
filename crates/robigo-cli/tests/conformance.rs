//! Differential conformance tests against GNU R, for the old runtime.
//!
//! Every `tests/r/**/*.R` script is run with the `robigo` binary and its
//! stdout, stderr and exit status compared byte-for-byte with GNU R
//! 4.5.0's (`*.Rout`, `*.Rerr`, `*.Rstatus`, recorded by
//! `scripts/regen-expected.sh`). The runner is `robigo-conformance`, which
//! robigo-rt's own conformance test shares (its column `RT`); this file
//! runs the column `OLD`, whose ratchet is `tests/r/passing/` (one empty
//! marker per script; `ROBIGO_CONF_UPDATE=1` adds markers).
//!
//! Environment: as `robigo_conformance` documents (ROBIGO_CONF_FILTER,
//! ROBIGO_CONF_DIFF, ROBIGO_CONF_UPDATE, ROBIGO_R_VERSION), plus
//!   ROBIGO_CONF_BIN=path        test this binary instead of cargo's
//!   ROBIGO_CONF_RESULTS=path    write the results of an unfiltered run in
//!                               scripts/corpus.py's result format (corpus
//!                               `conformance`), for the progress page

use robigo_conformance as conf;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The binary under test: cargo's, or `ROBIGO_CONF_BIN` so that CI tests the
/// release binary it ships while the harness builds in a cheaper profile.
fn robigo_bin() -> PathBuf {
    std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), PathBuf::from)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The `.requires` features this build has.
fn features() -> Vec<&'static str> {
    if cfg!(feature = "bytecode-fidelity") { vec!["bytecode-fidelity"] } else { Vec::new() }
}

#[test]
fn conformance() {
    let mut o = conf::Options::from_env(&conf::OLD, robigo_bin()).unwrap_or_else(|e| panic!("{e}"));
    o.features = features();
    o.results = std::env::var_os("ROBIGO_CONF_RESULTS").map(PathBuf::from);
    conf::run(&root(), &conf::OLD, &o).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn compiled_matches_interpreted() {
    let root = root();
    let scripts = conf::scripts(&root.join("tests/r"), &features());
    let bin = robigo_bin();
    let run = |script: &Path, compile: bool, jit: bool| {
        let out = conf::command(&bin, script)
            .env("ROBIGO_COMPILE", if compile { "1" } else { "0" })
            .env("ROBIGO_JIT", if jit { "1" } else { "0" })
            .output()
            .expect("spawn robigo");
        (out.stdout, out.stderr, out.status.code())
    };
    let mut bad = Vec::new();
    for s in &scripts {
        let interp = run(s, false, false);
        for (what, compile, jit) in [("VM", true, false), ("VM+JIT", true, true)] {
            if run(s, compile, jit) != interp {
                bad.push(format!("{} ({})", s.strip_prefix(&root).unwrap().display(), what));
            }
        }
    }
    assert!(bad.is_empty(), "compiled and interpreted runs differ for: {:?}", bad);
}

/// Compiled code must stay compiled: the guards that check `+`, `{`,
/// `<-` etc. still mean the base functions succeed on ordinary programs.
/// (A failing guard is still correct, just slow, so the other tests
/// would not notice.)
#[test]
fn vm_guards_hold() {
    let root = root();
    let script = root.join("benchmarks/suite/nbody.R");
    let out = Command::new(robigo_bin())
        .arg(&script)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("ROBIGO_COMPILE", "1")
        .env("ROBIGO_VM_TRACE", "1")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .output()
        .expect("spawn robigo");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("guard failed"), "compiled code fell back to the interpreter:\n{}", err);
}
