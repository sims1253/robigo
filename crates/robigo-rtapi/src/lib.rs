//! robigo-rtapi: GNU R's C API exported over `robigo-rt`, with no host
//! table (`docs/g3.md` §14.1). Each export collects on entry when due, roots
//! its SEXP arguments for its duration and turns a Rust error into
//! `R_run_onexits` and a longjmp (§3 item 6, §6.5).
//!
//! Empty until A1 fills it.

use robigo_rt as _;

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");
