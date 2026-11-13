//! Long compact sequences, unexpanded (issue #460).
//!
//! GNU R's `:`, `seq_len`, `seq_along` and `seq()` return ALTREP objects
//! (altclasses.c compact_intseq / compact_realseq) that hold a length, a
//! first value and an increment of +1 or -1, and expand only when C asks
//! for their data pointer. robigo stores every ordinary vector expanded
//! (`Vector::data` is a plain `Vec`, so element access costs nothing
//! extra). A sequence of [`LAZY_MIN_LEN`] or more elements is instead a
//! [`Value::Compact`]: shorter ones are expanded at once and registered as
//! compact (`COMPACT_INT`/`COMPACT_REAL` in `value.rs`), which is
//! observably the same.
//!
//! A `Value::Compact` is read without expanding by the operations GNU R
//! runs without a data pointer: `length`, `[`, `[[`, `for`, `sum`,
//! `min`/`max`, `serialize`, printing, `inspect`, `as.integer`/`as.double`
//! (which return it or a compact real sequence) and the predicates that
//! read only the type and attributes. Everything else sees an ordinary
//! vector: [`CompactSeq::expanded`] makes it once and caches it, and the
//! two places where values reach code that reads elements call it:
//! - every builtin, before its body runs (`Interp::invoke_builtin`,
//!   `crate::altrep::force_builtin_args`, unless the builtin is one of the
//!   readers above: `may_stay_compact`);
//! - every special that reads an operand (`crate::altrep::forced`).
//!
//! So no builtin body, container element or attribute holds one: a
//! `match` on a value that came through either point need not know the
//! variant. Code that matches values from elsewhere matches exhaustively
//! or calls [`Value::expand_compact`].

use std::cell::OnceCell;

use super::Value;
use crate::error::RResult;
use crate::memory::Elt;

/// The shortest sequence kept unexpanded. A shorter one is expanded at once
/// (4 MiB of integers at this length): its memory does not matter, and it
/// takes the ordinary vector paths.
pub const LAZY_MIN_LEN: usize = 1 << 20;

/// An unexpanded compact sequence: `n` elements from `first` by +1 or -1.
pub struct CompactSeq {
    n: usize,
    first: f64,
    down: bool,
    /// compact_intseq (else compact_realseq)
    int: bool,
    /// The expansion, made the first time code needs the elements
    /// (registered as a compact sequence, so it serializes as one).
    expanded: OnceCell<Value>,
}

impl CompactSeq {
    /// A sequence of `n` elements from `first` by `inc` (1 or -1). An
    /// integer sequence's elements all fit `i32`; a real sequence starts
    /// integral and below 2^52 in magnitude, and its elements are
    /// `first + inc * i` as GNU R computes them.
    pub(crate) fn new(n: usize, first: f64, inc: f64, int: bool) -> CompactSeq {
        debug_assert!(inc == 1.0 || inc == -1.0);
        debug_assert!(n >= 2);
        crate::altrep::note_compact(1);
        CompactSeq { n, first, down: inc < 0.0, int, expanded: OnceCell::new() }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn first(&self) -> f64 {
        self.first
    }

    /// The increment, 1 or -1.
    #[inline]
    pub fn inc(&self) -> f64 {
        if self.down { -1.0 } else { 1.0 }
    }

    /// The last element.
    pub fn last(&self) -> f64 {
        self.at(self.n - 1)
    }

    /// Whether this is a compact_intseq (else a compact_realseq).
    #[inline]
    pub fn is_int(&self) -> bool {
        self.int
    }

    pub fn vtype(&self) -> super::VType {
        if self.int { super::VType::Int } else { super::VType::Real }
    }

    /// The elements' allocation type.
    pub fn elt_type(&self) -> Elt {
        if self.int { Elt::Int } else { Elt::Real }
    }

    /// Element `i` (0-based, in bounds) as a double.
    // R: altclasses.c compact_intseq_Elt / compact_realseq_Elt: n1 + inc * i
    #[inline]
    pub fn at(&self, i: usize) -> f64 {
        self.first + self.inc() * i as f64
    }

    /// Element `i` (0-based, in bounds) as a length-one vector.
    pub fn elt(&self, i: usize) -> Value {
        if self.int { Value::int(self.at(i) as i32) } else { Value::real(self.at(i)) }
    }

    /// The first `k` (at most the length) elements as an ordinary vector.
    pub fn head(&self, k: usize) -> Value {
        let k = k.min(self.n);
        if self.int {
            Value::ints((0..k).map(|i| self.at(i) as i32).collect())
        } else {
            Value::reals((0..k).map(|i| self.at(i)).collect())
        }
    }

    /// Elements at 1-based positions, NA where a position is NA (`None`)
    /// or past the end, as an ordinary vector (R: ExtractSubset on an
    /// ALTREP object, which reads element by element).
    pub fn extract(&self, idx: impl ExactSizeIterator<Item = Option<usize>>) -> Value {
        let n = self.n;
        if self.int {
            Value::ints(
                idx.map(|p| match p {
                    Some(p) if (1..=n).contains(&p) => self.at(p - 1) as i32,
                    _ => crate::value::NA_INTEGER,
                })
                .collect(),
            )
        } else {
            Value::reals(
                idx.map(|p| match p {
                    Some(p) if (1..=n).contains(&p) => self.at(p - 1),
                    _ => crate::value::NA_REAL,
                })
                .collect(),
            )
        }
    }

    /// The sequence as an ordinary vector (registered as compact), or
    /// allocVector's catchable error when its elements cannot be had.
    pub fn expanded(&self) -> RResult<Value> {
        if let Some(v) = self.expanded.get() {
            return Ok(v.clone());
        }
        crate::memory::check_vector(self.n, self.elt_type())?;
        Ok(self.expanded.get_or_init(|| self.expand()).clone())
    }

    /// [`Self::expanded`] where no caller can take an R error: the process
    /// ends with allocVector's message when the elements cannot be had.
    pub fn expanded_or_exit(&self) -> Value {
        match self.expanded() {
            Ok(v) => v,
            Err(_) => crate::memory::fatal_cannot_allocate(self.n, self.elt_type()),
        }
    }

    #[cold]
    fn expand(&self) -> Value {
        let (n, inc) = (self.n, self.inc());
        if self.int {
            let first = self.first as i64;
            let step = inc as i64;
            super::register_compact(Value::ints((0..n as i64).map(|k| (first + k * step) as i32).collect()))
        } else {
            super::register_compact(Value::reals((0..n).map(|k| self.first + inc * k as f64).collect()))
        }
    }
}

impl Drop for CompactSeq {
    fn drop(&mut self) {
        crate::altrep::note_compact(-1);
    }
}

/// Expand the compact sequences among `items`, the elements of a list a
/// builtin makes from values it did not get as arguments (function results,
/// variables): no container holds one. One load while none lives.
#[inline(always)]
pub fn expand_items(items: &mut [Value]) -> RResult<()> {
    if crate::altrep::any_lazy() {
        expand_items_slow(items)?;
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn expand_items_slow(items: &mut [Value]) -> RResult<()> {
    for v in items {
        if let Value::Compact(c) = v {
            *v = c.expanded()?;
        }
    }
    Ok(())
}

impl Value {
    /// `self`, an ordinary vector if it was a [`Value::Compact`] (see the
    /// module documentation for where this happens on its own).
    #[inline]
    pub fn expand_compact(self) -> RResult<Value> {
        match self {
            Value::Compact(c) => c.expanded(),
            v => Ok(v),
        }
    }

    /// `self`, expanded if it is a [`Value::Compact`]: one load while no
    /// compact sequence lives.
    #[inline(always)]
    pub fn expand_lazy(self) -> RResult<Value> {
        if crate::altrep::any_lazy() { self.expand_compact() } else { Ok(self) }
    }
}
