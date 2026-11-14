//! robigo-rt's own conformance (docs/g3.md L1, §14.3): `tests/rt/**/*.R`,
//! and the `tests/r` cases it has ratcheted, against GNU R 4.6.2's
//! recordings (`<suite>/R-4.6.2/`), with its own ratchet column
//! `<suite>/passing-rt/`. The runner is `robigo-conformance`'s column `RT`,
//! which robigo-cli's conformance test shares (column `OLD`).
//!
//!   cargo test --release -p robigo-rtapi --test conformance -- --nocapture
//!   ROBIGO_CONF_UPDATE=1 ...    add markers for the cases that pass
//!   ROBIGO_CONF_SWEEP=1 ...     also run every tests/r case, not only the
//!                               ratcheted ones
//!   ROBIGO_RT_BIN=path          test this binary instead of cargo's
//!
//! and ROBIGO_CONF_FILTER, ROBIGO_CONF_DIFF and ROBIGO_R_VERSION as
//! `robigo_conformance` documents. The binary variable is rt's own: CI's
//! `test` job points `ROBIGO_CONF_BIN` at the old runtime's binary for the
//! whole workspace.

use robigo_conformance as conf;
use std::path::{Path, PathBuf};

// Rebuild when another checkout built this test last (.cargo/config.toml):
// the repository root below is compiled in.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

fn rt_bin() -> PathBuf {
    std::env::var_os("ROBIGO_RT_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo-rt").into(), PathBuf::from)
}

#[test]
fn rt_conformance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let o = conf::Options::from_env(&conf::RT, rt_bin()).unwrap_or_else(|e| panic!("{e}"));
    conf::run(&root, &conf::RT, &o).unwrap_or_else(|e| panic!("{e}"));
}
