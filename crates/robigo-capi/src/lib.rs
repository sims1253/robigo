//! `libR.so`: GNU R 4.5's C API over robigo-core (`docs/core.md`).
//!
//! One module per GNU R source file whose API it exports. Everything that
//! lives in the host runtime (evaluation, environments, conditions,
//! printing) goes through the callback table in [`host`]. Census symbols
//! that are not implemented yet are exported by [`stubs`] as functions
//! that raise "robigo: C API function 'X' is not implemented yet".

#![allow(non_snake_case, non_upper_case_globals, non_camel_case_types, clippy::missing_safety_doc, static_mut_refs)]
#![allow(unsafe_op_in_unsafe_fn)]

pub mod altrep;
pub mod appl;
pub mod arith;
pub mod attrib;
pub mod bridge;
pub mod coerce;
pub mod connections;
pub mod dotcode;
pub mod envir;
pub mod errors;
pub mod extptr;
pub mod host;
pub mod list;
pub mod memory;
pub mod nmath;
pub mod rdynload;
pub mod rng;
pub mod serialize;
pub mod sort;
pub mod state;
pub mod strings;
pub mod stubs;
pub mod util;

pub use robigo_core::Sexp;
pub type SEXP = Sexp;
pub type R_xlen_t = isize;
pub type Rboolean = std::ffi::c_int;

/// How many contexts (`setjmp` frames) are active; 0 between calls.
pub fn unwind_depth() -> usize {
    robigo_core::unwind::depth()
}
