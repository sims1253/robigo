//! The robigo R runtime: object model, evaluator and builtins.

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
pub mod jit;
pub mod lang;
pub mod memory;
pub mod native;
pub use robigo_numerics as nmath;
pub mod print;
pub mod rdbflat;
pub mod reachability;
pub mod s4;
pub mod serialize;
pub mod startup;
pub mod sym;
pub mod task;
pub mod typeprof;
pub mod value;
pub mod vm;

pub use env::Env;
pub use error::{Flow, RResult};
pub use interp::Interp;
pub use sym::Sym;
pub use value::Value;
