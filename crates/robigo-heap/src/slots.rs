//! The word map of a slot frame (docs/g3.md §5.1; m1-plan N1b): an
//! unhashed environment whose bindings are typed slots inside the ENVSXP
//! itself, plus an overflow block for names outside its shape. The one
//! place the map's kinds and meta bits are written down (its offsets are
//! [`crate::layout::slots`]'): the collector's trace arm
//! ([`crate::Sexp::trace_fields`]) and the runtime (`robigo-rtcore`'s
//! `env::frame::layout`) both read it from here, so a measured variant
//! (m1-plan X1, D2) swaps this module and nothing else.
//!
//! g3.md §5's three constraints on the map: the plain-value read is one
//! test on the key ([`KIND_MASK`] bits 0, [`VALUE`]); unbound and cell are
//! out of line (kinds of their own, off the fast path); the definition
//! stamp lives outside the key (in the slot's meta word).
//!
//! The words (offsets in [`crate::layout::slots`]): the overflow block,
//! ENCLOS, the shape, the version, the caller, the meta word, then the
//! slots, each a key, a value, an aux word and a meta word.
//!
//! Symbols are 16-byte aligned (M1's heap; M2's arena aligns them to 64),
//! so the four low bits of a key are the kind. A slot that was never keyed
//! has key [`UNBOUND`] (a null symbol). Which words of a slot the collector
//! traces is a function of its kind alone ([`val_traced`], [`aux_traced`]):
//! that is the whole tracing rule.

use crate::layout::node;
pub use crate::layout::slots::*;

/// The header flag (byte +1) of an ENVSXP laid out as a slot frame (or an
/// overflow block): [`crate::flags::SLOTS`].
pub use crate::flags::SLOTS;

/// A key's kind bits.
pub const KIND_MASK: u64 = 0xF;

// The kinds (docs/g3.md §5.1, §6.3). Missing (no default) is a VALUE slot
// holding R_MissingArg with MISSING 1; missing-default is a DELAYED slot of
// ORIGIN self with MISSING 2; locked is a flag on any kind.
/// A plain object value: the one-test fast path.
pub const VALUE: u64 = 0;
/// An unboxed double in `val` (`R_expand_binding_value` boxes it).
pub const REAL: u64 = 1;
/// An unboxed integer in the low half of `val`.
pub const INT: u64 = 2;
/// An unboxed logical in the low half of `val`.
pub const LGL: u64 = 3;
/// Reserved by the shape and never defined, or removed by `rm`: lookup
/// continues in the enclosure, and listing does not see it.
pub const UNBOUND: u64 = 4;
/// An argument not yet evaluated: `aux` its expression; `val` a value the
/// caller computed early (§6.2's precomputed argument) or null; the
/// environment is the frame's caller (ORIGIN caller) or the frame itself
/// (ORIGIN self: a default). PRSEEN and MISSING in the meta flags.
pub const DELAYED: u64 = 5;
/// An evaluated argument: `val` its value, `aux` its expression.
pub const FORCED: u64 = 6;
/// An active binding: `val` the function.
pub const ACTIVE: u64 = 7;
/// Cell-backed (`R_findVarLocInFrame` made it so, for good): `val` is a
/// GNU R binding cell (a LISTSXP whose CAR is the value and gp the binding
/// bits), and every read and write goes through it.
pub const CELL: u64 = 8;
/// An S3 method's argument shared with its generic (GNU R's method binds
/// the generic's own promise): `val` the generic's frame (traced, not
/// counted), `aux` the index of its argument slot (raw bits). That slot is
/// DELAYED or FORCED and has [`META_ALIASED`]; a read through the alias is
/// a read of it, and anything that needs the binding as an object makes it
/// the argument's promise first (robigo-rtcore's `env::alias`).
pub const ALIAS: u64 = 9;

/// Slot meta flags (bits 32 and up; the stamp is bits 0-31).
pub const META_LOCKED: u64 = 1 << 32;
/// GNU R's `ASSIGNMENT_PENDING` for this binding.
pub const META_PENDING: u64 = 1 << 33;
/// GNU R's `MISSING` (0, 1, 2), two bits.
pub const META_MISSING_SHIFT: u32 = 34;
/// GNU R's `PRSEEN` of a delayed slot (0, 1 under evaluation, 2
/// interrupted), two bits.
pub const META_PRSEEN_SHIFT: u32 = 36;
/// A delayed slot evaluates in the frame itself (a default argument), not
/// in the caller's environment.
pub const META_ORIGIN_SELF: u64 = 1 << 38;
/// A delayed slot's expression is byte code's: compiled code made the
/// argument, so GNU R's promise would hold a BCODESXP as PRCODE and run it
/// by `bcEval` (robigo-rt evaluates the expression under its `Bc`
/// profile). Kept when the slot becomes a promise object.
pub const META_BYTECODE: u64 = 1 << 39;
/// A delayed or forced argument slot some method's [`ALIAS`] reads: a write,
/// removal, materialization or cell conversion of the slot makes its
/// aliases the argument's promise first. Beside [`META_LOCKED`], so a
/// writer's fast path tests both at once.
pub const META_ALIASED: u64 = 1 << 40;
pub const META_STAMP: u64 = 0xFFFF_FFFF;

/// A slot frame's header `extra` bit: the closure call that made the frame
/// is still running ([`crate::extra::env::CALL_LIVE`], in the environment's
/// `extra` map, so none overlaps another; it was 1 << 4 here, which is
/// `extra::SEEN_BY_C`).
pub use crate::extra::env::CALL_LIVE;

/// The vector cells (8-byte VECRECs) the heap's accounting charges a slot
/// frame beyond its node: the frame words and slots past a node's size.
#[inline]
pub const fn frame_cells(capacity: usize) -> usize {
    (frame_bytes(capacity) - node::size(crate::types::ENVSXP)) / 8
}

/// Whether a slot of `kind` holds an object in `val` (null allowed: an
/// unbound slot's and a delayed slot's without a precomputed value).
#[inline(always)]
pub const fn val_traced(kind: u64) -> bool {
    matches!(kind, VALUE | DELAYED | FORCED | ACTIVE | CELL | ALIAS)
}

/// Whether a slot of `kind` holds an object in `aux`.
#[inline(always)]
pub const fn aux_traced(kind: u64) -> bool {
    matches!(kind, DELAYED | FORCED)
}
