//! The robigo R runtime: object model, evaluator and builtins.

// Unsafe code is limited to the modules marked below and the few statements
// marked in place (`docs/architecture.md`, "Unsafe code").
#![deny(unsafe_code)]

pub mod altrep;
pub mod attrib;
pub mod bootstrap;
pub mod builtins;
pub mod coerce;
pub mod compile;
pub mod conditions;
pub mod cpoly;
pub mod deparse;
pub mod dispatch;
pub mod env;
pub mod error;
pub mod format;
pub mod interp;
#[allow(unsafe_code)] // enters generated machine code
pub mod jit;
pub mod lang;
pub mod memory;
pub mod native;
pub use robigo_numerics as nmath;
pub mod print;
pub mod rdbflat;
pub mod reachability;
pub mod s4;
#[allow(unsafe_code)] // the fixed-width readers, measured against safe code
pub mod serialize;
pub mod startup;
pub mod sym;
#[allow(unsafe_code)] // the C library
pub(crate) mod sys;
pub mod task;
pub mod typeprof;
#[allow(unsafe_code)] // `RString`, and C-level writes through `Rc` aliases
pub mod value;
#[allow(unsafe_code)] // uninitialized register files, in-place arithmetic kernels
pub mod vm;

pub use env::Env;
pub use error::{Flow, RResult};
pub use interp::Interp;
pub use sym::Sym;
pub use value::Value;
