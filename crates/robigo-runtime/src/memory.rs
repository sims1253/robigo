//! R's "cannot allocate vector of size ..." error.
//!
//! Rust aborts the process when an infallible allocation fails, and stable
//! Rust has no way to turn that into an unwinding error, so a failed
//! `vec![...; n]` cannot be caught by `tryCatch`. Only requests whose size
//! comes from a number can be arbitrarily large: an operation sized by an
//! existing vector asks for no more than R code already holds. The places
//! that turn a number into a length (`vector()`, `:`, `rep()`, unserialize,
//! ...) therefore check here first.

use crate::error::{Flow, RResult};
use crate::value::{Complex, Lgl, RStr, Value};

/// Requests below this go straight to the allocator: the probe would cost
/// a call on every small vector to catch a failure that does not happen.
const PROBE_BYTES: usize = 1 << 30;

/// R: Rinternals.h:R_XLEN_T_MAX.
const R_XLEN_T_MAX: usize = 1 << 52;

/// R's element sizes, which the message reports, against the larger ones
/// robigo stores for strings and lists.
#[derive(Clone, Copy)]
pub enum Elt {
    Lgl,
    Int,
    Real,
    Cplx,
    Str,
    Raw,
    List,
}

impl Elt {
    pub fn of(x: &Value) -> Elt {
        match x {
            Value::Lgl(_) => Elt::Lgl,
            Value::Int(_) => Elt::Int,
            Value::Real(_) => Elt::Real,
            Value::Cplx(_) => Elt::Cplx,
            Value::Str(_) => Elt::Str,
            Value::Raw(_) => Elt::Raw,
            _ => Elt::List,
        }
    }

    fn r_size(self) -> usize {
        match self {
            Elt::Lgl | Elt::Int => 4,
            Elt::Real | Elt::Str | Elt::List => 8,
            Elt::Cplx => 16,
            Elt::Raw => 1,
        }
    }

    fn rust_size(self) -> usize {
        match self {
            Elt::Lgl => size_of::<Lgl>(),
            Elt::Int => size_of::<i32>(),
            Elt::Real => size_of::<f64>(),
            Elt::Cplx => size_of::<Complex>(),
            Elt::Str => size_of::<RStr>(),
            Elt::Raw => size_of::<u8>(),
            Elt::List => size_of::<Value>(),
        }
    }
}

/// R: memory.c:allocVector3's error.
fn cannot_allocate(n: usize, r_size: usize) -> Flow {
    // R rounds the request up to whole 8-byte VECRECs
    let bytes = ((n as f64) * (r_size as f64) / 8.0).ceil() * 8.0;
    let dsize = bytes / 1024.0;
    Flow::error_nocall(if dsize > 1024.0 * 1024.0 {
        format!("cannot allocate vector of size {:.1} Gb", dsize / 1024.0 / 1024.0)
    } else if dsize > 1024.0 {
        format!("cannot allocate vector of size {:.1} Mb", dsize / 1024.0)
    } else {
        format!("cannot allocate vector of size {:.0} Kb", dsize)
    })
}

/// Fail like R if `n` elements of `elt` cannot be allocated. The block is
/// released again: the caller allocates it with `vec!`, which can stay
/// calloc-backed, so a large `numeric(n)` does not touch its pages.
#[inline]
pub fn check_vector(n: usize, elt: Elt) -> RResult<()> {
    if n.saturating_mul(elt.rust_size()) < PROBE_BYTES {
        return Ok(());
    }
    probe_vector(n, elt)
}

#[cold]
fn probe_vector(n: usize, elt: Elt) -> RResult<()> {
    if n > R_XLEN_T_MAX {
        return Err(Flow::error("vector is too large"));
    }
    let probed = n.checked_mul(elt.rust_size()).is_some_and(|b| Vec::<u8>::new().try_reserve_exact(b).is_ok());
    if probed { Ok(()) } else { Err(cannot_allocate(n, elt.r_size())) }
}

/// A vector of capacity `n`, failing like R's allocVector. Unserialize makes
/// one per vector it reads, so the common size stays one inlined allocation.
#[inline]
pub fn try_vec<T>(n: usize, elt: Elt) -> RResult<Vec<T>> {
    if n.saturating_mul(size_of::<T>()) < PROBE_BYTES {
        return Ok(Vec::with_capacity(n));
    }
    try_vec_large(n, elt)
}

#[cold]
fn try_vec_large<T>(n: usize, elt: Elt) -> RResult<Vec<T>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(cannot_allocate(n, elt.r_size()));
    }
    Ok(v)
}
