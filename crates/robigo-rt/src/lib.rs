//! robigo-rt: G3's runtime (`docs/g3.md`): the evaluator and its two
//! profiles, contexts and unwinding, S3 dispatch, the condition system and
//! the loader of the base R subset (`proto/base-subset.txt`).
//!
//! Empty until M1's packages fill it. The `robigo` binary does not depend
//! on this crate (`scripts/rt-ci.py` checks it), which is why pull requests
//! that touch only the runtime's crates run CI's `rt` slice.

use robigo_rtbuiltins as _;
use robigo_rtcore as _;

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");
