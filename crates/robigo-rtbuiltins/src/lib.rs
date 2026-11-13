//! robigo-rtbuiltins: G3's `.Primitive` and `.Internal` layer, ported from
//! GNU R 4.6.2's C sources one module per file (`docs/g3.md` §10), over the
//! safe facade of `robigo-rtcore` (§2.4).
//!
//! - [`registry`]: the builtins, one row each with names.c's columns
//!   (`code`, the `eval` digits, arity, deparse info) and an [`Effect`]
//!   class for differential mode. A builtin is a [`BuiltinFn`]:
//!   `fn(cx, call, args, env, out)`.
//! - [`kernel`]: GNU R's arithmetic, comparison and logic on plain data,
//!   shared with the VM's ops (one implementation, §10).
//! - The builtins, by GNU R file: [`arith`] (arithmetic.c), [`relop`]
//!   (relop.c), [`logic`] (logic.c), [`paste`] (paste.c), [`character`]
//!   (character.c), [`envir`] (envir.c).
//! - Their support: [`attrib`] (attrib.c's reads and the order a result's
//!   attributes are built in), [`coerce`] (to character), [`text`]
//!   (`Seql`, `Scollate`, translation), [`util`].
//!
//! A builtin reaches the evaluator, the dispatcher and the condition system
//! through the isolate's hook table (`cx.eval`, `cx.apply`,
//! `cx.try_dispatch`, `cx.force`, `cx.warning_call`), which `rt` installs.
//!
//! No `unsafe` here, ever: what a builtin cannot say safely belongs in the
//! facade.
#![forbid(unsafe_code)]

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

pub mod arith;
pub mod attrib;
pub mod character;
pub mod coerce;
pub mod envir;
pub mod is;
pub mod kernel;
pub mod logic;
pub mod paste;
pub mod registry;
pub mod relop;
pub mod seq;
pub mod subassign;
pub mod subscript;
pub mod subset;
pub mod text;
pub mod util;

pub use registry::{BUILTINS, Builtin, BuiltinFn, Effect, PpInfo, PpKind, Prec, Visibility};
