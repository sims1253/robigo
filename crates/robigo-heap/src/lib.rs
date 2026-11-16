//! robigo-heap: R objects as header'd heap objects behind one pointer (a
//! `SEXP`), on a non-moving mark-region heap (a debug mode moves them). The memory manager of G3's
//! runtime (`docs/g3.md`, the appendix's heap rows).
//!
//! The collection protocol (docs/g3.md §3, H2): allocation never collects;
//! it sets [`GC_DUE`] in a poll word, or refuses with [`NeedGc`] what only
//! a collection could serve. Collections run where the embedder asks, with
//! its roots as a [`Trace`] value: at a safepoint ([`Core::safepoint`]),
//! before an allocation that may collect ([`Core::alloc_rooted`],
//! [`Core::make_room`]), or explicitly ([`Core::collect`]). Their order is
//! precise roots, weak references and finalizers decided on them alone, C
//! segments as pins (L8), the CHARSXP cache, the sweep; the conservative
//! scan reads only C segments, the frames of package code between a native
//! entry and the export it called, never a Rust frame. `ROBIGO_GC_STATS`
//! reports collections per safepoint kind, E4's overshoot and allocations by
//! type ([`StatsMode`]).
//!
//! The debug collectors (docs/g3.md §3 "Checks", H3): `ROBIGO_GC_MOVE=1`
//! moves every object it may at every collection and poisons the old copy
//! ([`POISON_TYPE`]); `ROBIGO_GC_PIN_REPORT` names the weak references
//! decided dead while a C word reaches their key, with the routine
//! ([`PinReportTo`]); `ROBIGO_GC_STRONG_DLL` confirms a report.
//!
//! A fork of `crates/robigo-core` at main `25bad70835a1c9bd19c70ffb26b1061987b7873a`
//! (2026-11-13; robigo-core last changed in `be7fcd3b`), with the appendix's
//! *removed* row deleted: everything that existed only for the old
//! runtime's native bridge (the list is in `docs/ledger/g3.md`, H1). Of
//! experiment A's switches, `ROBIGO_CALL65_PLAIN` stays; its order
//! (`ROBIGO_GC_PIN_ONLY`) is the only one since lead decision L8.
//! robigo-core stays as it was for the old
//! runtime and `robigo-capi`; the two crates share no code, and their C
//! shims export distinct symbols (`robigo_heap_*` here), so both can link
//! into one binary.
//!
//! This crate knows memory.c-level R: object layout, allocation,
//! collection, CHARSXP interning and symbols. R semantics above that
//! (attributes, coercion, environments, evaluation) live in the rt crates
//! that use it.

pub mod chars;
mod gc;
mod heap;
mod heapsize;
pub mod layout;
pub mod slots;
mod trace;
pub mod unwind;

pub use gc::{
    AllocError, BuildFx, Core, GC_DUE, GcStats, HeapReport, LARGE, NeedGc, POISON_TYPE, PinReportTo, ReleaseFn,
    Safepoint, ScanSegment, ScanState, StatsMode, overshoot_bound,
};
pub use robigo_heap_derive::Trace;
pub use trace::{Trace, Visitor};

/// Which heap an object belongs to ([`Sexp::heap_id`], [`Core::id`]): the
/// owner checks of G3's runtime compare the two (docs/g3.md §2.3), so that
/// no isolate roots or stores another isolate's object.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HeapId(usize);

pub mod types;
use types::*;

pub use layout::{elt_size, extra, flags, gp};

/// GNU R's `cetype_t`.
pub mod ce {
    pub const NATIVE: i32 = 0;
    pub const UTF8: i32 = 1;
    pub const LATIN1: i32 = 2;
    pub const BYTES: i32 = 3;
    pub const SYMBOL: i32 = 5;
    pub const ANY: i32 = 99;
}

/// One R object: a pointer to its header. The same value C code has as
/// `SEXP`.
///
/// A `Sexp` is a plain address and says nothing about what it points at, so
/// every accessor that reads through it is `unsafe`. Their contracts share
/// one word: a *live* object is one a [`Core`] allocated and has not swept.
/// Liveness is not a property of the `Sexp`: an object stays live only while
/// the rooting rule (docs/core.md section 3) is met for it, so an unrooted one
/// can be freed by anything that allocates. Every accessor reads or writes
/// the fixed layout of [`layout`], and a store through one of
/// the `_raw` setters is invisible to the reference counts and the barrier.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Sexp(pub *mut u8);

/// GNU R's `Rcomplex`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rcomplex {
    pub r: f64,
    pub i: f64,
}

pub const NA_INTEGER: i32 = i32::MIN;
pub const NA_LOGICAL: i32 = i32::MIN;
/// GNU R's NA_real_: a NaN with payload 1954 (arithmetic.c R_ValueOfNA).
pub const NA_REAL: f64 = f64::from_bits(0x7FF0_0000_0000_07A2);

#[inline]
pub fn is_vector_type(t: u8) -> bool {
    matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP | CHARSXP | WEAKREFSXP)
}

impl Sexp {
    pub const NULL: Sexp = Sexp(std::ptr::null_mut());

    #[inline(always)]
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
    #[inline(always)]
    pub fn addr(self) -> usize {
        self.0 as usize
    }
    /// The word at byte offset `off`.
    ///
    /// # Safety
    /// `self` must be live and `off` inside its allocation.
    #[inline(always)]
    pub(crate) unsafe fn word(self, off: usize) -> *mut usize {
        // SAFETY: the caller keeps `off` inside the object's allocation.
        unsafe { self.0.add(off) as *mut usize }
    }
    /// The pointer field at byte offset `off`.
    ///
    /// # Safety
    /// `self` must be live, and `off` a pointer-sized field of its body. A
    /// word that is not an object (an EXTPTRSXP's car) comes back as a
    /// `Sexp` that must not be dereferenced.
    #[inline(always)]
    pub unsafe fn sexp_at(self, off: usize) -> Sexp {
        // SAFETY: the caller names a field of a live object; objects are
        // 16-byte aligned, so a field at a multiple of 8 is aligned.
        unsafe { Sexp(*(self.word(off) as *const *mut u8)) }
    }
    /// Store into the pointer field at byte offset `off`, with no reference
    /// count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::sexp_at`]. If the collector traces that field, `v`
    /// must be null or live, and on an immortal `self` the store needs
    /// the barrier ([`Sexp::store`]) or `v` is not kept alive.
    #[inline(always)]
    pub(crate) unsafe fn set_sexp_at(self, off: usize, v: Sexp) {
        // SAFETY: as in `sexp_at`; what `v` must be is the caller's contract.
        unsafe { *(self.word(off) as *mut *mut u8) = v.0 }
    }

    /// The heap that owns this object: one load from its block header (a
    /// large object's chunk has the same field), equal to that heap's
    /// [`Core::id`].
    ///
    /// # Safety
    /// `self` must be a live object of a live heap.
    #[inline(always)]
    pub unsafe fn heap_id(self) -> HeapId {
        // SAFETY: the caller's contract is `remembered_of`'s: every block and
        // large chunk header points at its heap's remembered set.
        HeapId(unsafe { heap::remembered_of(self.0) } as usize)
    }


    // -- the store (council item 8)
    //
    // Every store of an object into a traced field of a heap object, from
    // outside this crate, goes through `store`: GNU R's SETCAR shape
    // (memory.c: CHECK_OLD_TO_NEW, then FIX_REFCNT, then the write). The raw
    // setters above are this crate's own (construction, the collector).
    // What still bypasses it is C writing through a payload pointer it was
    // handed (`DATAPTR`/`STRING_PTR`/`VECTOR_PTR` on a STRSXP or VECSXP), as
    // in GNU R; a generational collector treats those objects as always
    // dirty (flags::SPARE, docs/core.md §11).

    /// Store `v` into the field `slot` of `self`: the barrier, then GNU R's
    /// reference counts (`FIX_REFCNT`: the new value gains a reference, the
    /// old one loses it, if `self` tracks references), then the write.
    ///
    /// # Safety
    /// `self` must be live, and `slot` a traced pointer field of its body
    /// (an attribute slot that is not a CHARSXP's hash, a node field that
    /// contains an object, a STRSXP/VECSXP/EXPRSXP/WEAKREFSXP element within
    /// its length). `v` must be null or live, and so must the field's old
    /// value.
    // R: memory.c:SETCAR (and SETCDR, SET_TAG, SET_ATTRIB, SET_VECTOR_ELT)
    #[inline(always)]
    pub unsafe fn store(self, slot: *mut Sexp, v: Sexp) {
        // SAFETY: the caller's contract: `self` is live, `slot` is one of its
        // traced fields, and the old and new values are null or live.
        unsafe {
            // the slow path does the whole store (a tail call), so the
            // fast path keeps no registers across a call
            if self.flags() & flags::BARRIER != 0 {
                return gc::store_slow(self, slot, v);
            }
            self.store_counted(slot, v);
        }
    }
    /// [`Sexp::store`] without counts: the barrier and the write. For a
    /// traced word whose reference counts nothing (an argument alias's
    /// frame, robigo-rtcore's `env::alias`).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; the old value counted nothing either.
    #[inline(always)]
    pub unsafe fn store_uncounted(self, slot: *mut Sexp, v: Sexp) {
        // SAFETY: the caller's contract.
        unsafe {
            if self.flags() & flags::BARRIER != 0 {
                gc::barrier_only(self);
            }
            *slot = v;
        }
    }
    /// The store after the barrier: GNU R's FIX_REFCNT and the write.
    ///
    /// # Safety
    /// As for [`Sexp::store`].
    // R: memory.c:FIX_REFCNT
    #[inline(always)]
    pub(crate) unsafe fn store_counted(self, slot: *mut Sexp, v: Sexp) {
        // SAFETY: as for `store`.
        unsafe {
            let old = *slot;
            if old != v && self.tracks_refs() {
                if !old.is_null() {
                    old.decref();
                }
                if !v.is_null() {
                    v.incref();
                }
            }
            *slot = v;
        }
    }

    // -- reference counts (copy-on-modify, GNU R's REFCNT protocol)
    /// One more reference; a saturated count stays put (GNU R's REFCNTMAX).
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn incref(self) {
        // SAFETY: `self` is live, all these header accesses need.
        unsafe {
            if self.has_flag(flags::IMMORTAL) {
                return;
            }
            let c = self.refcnt();
            if c < u16::MAX {
                self.set_refcnt(c + 1);
            }
        }
    }
    /// One reference fewer; a saturated count stays put.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn decref(self) {
        // SAFETY: `self` is live, all these header accesses need.
        unsafe {
            if self.has_flag(flags::IMMORTAL) {
                return;
            }
            let c = self.refcnt();
            if c > 0 && c < u16::MAX {
                self.set_refcnt(c - 1);
            }
        }
    }
}

/// An environment's body (council item 20): only ENCLOS has a fixed
/// offset ([`layout::env::OFF_ENCLOS`]). FRAME and HASHTAB are shims over the frame
/// words (the others of [`layout::env`]), so the runtime is free to change what those contain
/// (G3's slot frames): then `frame` builds the pairlist of bindings that C
/// expects, and `hashtab` the hashed frame, from it. No package imports
/// FRAME or HASHTAB (`tests/capi/symbols.tsv`); C reaches an environment's
/// bindings through `Rf_findVarInFrame` & co.
///
/// A hashed environment's frame words contain GNU R's representation:
/// `OFF_FRAME` the frame (R_NilValue), `OFF_HASHTAB` the hash table (a VECSXP of chains), `OFF_VERSION` the
/// runtime's version box or R_NilValue. An unhashed one is a slot frame
/// ([`crate::flags::SLOTS`], [`crate::slots`]): `frame` and `hashtab` do not
/// apply to it (the runtime builds FRAME's snapshot from the slots), and
/// debug builds assert so.
///
/// Every function here takes a live, node-shaped ENVSXP (from
/// [`Core::alloc_node`]) and, for a store, a live value.
pub mod env {
    use crate::Sexp;
    use crate::layout::env::{OFF_ENCLOS, OFF_FRAME, OFF_HASHTAB};

    /// GNU R's ENCLOS.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    #[inline(always)]
    pub unsafe fn enclos(x: Sexp) -> Sexp {
        // SAFETY: the caller's contract: ENCLOS is at OFF_ENCLOS.
        unsafe { x.sexp_at(OFF_ENCLOS) }
    }
    /// GNU R's SET_ENCLOS.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP, `v` a live environment or R_NilValue.
    #[inline(always)]
    pub unsafe fn set_enclos(x: Sexp, v: Sexp) {
        // SAFETY: as above; the store keeps counts and the barrier.
        unsafe { x.store(x.field(OFF_ENCLOS), v) }
    }
    /// GNU R's FRAME: the bindings as a pairlist of tagged cells.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    // R: memory.c:FRAME
    pub unsafe fn frame(x: Sexp) -> Sexp {
        // SAFETY: the frame representation above: the pairlist at `OFF_FRAME`.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: FRAME of a slot frame (its FRAME word is the overflow)");
            x.sexp_at(OFF_FRAME)
        }
    }
    /// GNU R's SET_FRAME.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP, `v` a live pairlist or R_NilValue.
    // R: memory.c:SET_FRAME
    pub unsafe fn set_frame(x: Sexp, v: Sexp) {
        // SAFETY: as above.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: SET_FRAME of a slot frame");
            x.store(x.field(OFF_FRAME), v)
        }
    }
    /// GNU R's HASHTAB: the hashed frame, or R_NilValue for an unhashed one.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    // R: memory.c:HASHTAB
    pub unsafe fn hashtab(x: Sexp) -> Sexp {
        // SAFETY: the frame representation above: the table at `OFF_HASHTAB`.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: HASHTAB of a slot frame (its HASHTAB word is the shape)");
            x.sexp_at(OFF_HASHTAB)
        }
    }
    /// GNU R's SET_HASHTAB.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP, `v` a live VECSXP or R_NilValue.
    // R: memory.c:SET_HASHTAB
    pub unsafe fn set_hashtab(x: Sexp, v: Sexp) {
        // SAFETY: as above.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: SET_HASHTAB of a slot frame");
            x.store(x.field(OFF_HASHTAB), v)
        }
    }
}

/// GNU R's `R_Newhashpjw` (envir.c, the Dragon Book's hashpjw): the hash
/// of a name that the symbol table, hashed environments and saved images
/// use, so listing orders depend on it. Each byte enters as C's `char`, which
/// is signed on x86-64 and unsigned on aarch64 Linux, as GNU R's does. The
/// result is below 2^28: the top nibble is folded away at every step.
// R: envir.c:R_Newhashpjw
#[inline]
pub fn sym_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        // `(*p)` is a `char`, converted to unsigned for the addition
        h = (h << 4).wrapping_add(b as std::ffi::c_char as i32 as u32);
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
            h ^= g;
        }
    }
    h
}

/// The 32-bit hash the CHARSXP cache keys on (FxHash over the bytes).
#[inline]
pub fn hash_bytes(b: &[u8]) -> u32 {
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
    let mut h: u64 = 0;
    let mut add = |w: u64| h = (h.rotate_left(5) ^ w).wrapping_mul(SEED);
    let mut s = b;
    while s.len() >= 8 {
        add(u64::from_le_bytes(s[..8].try_into().unwrap()));
        s = &s[8..];
    }
    if s.len() >= 4 {
        add(u32::from_le_bytes(s[..4].try_into().unwrap()) as u64);
        s = &s[4..];
    }
    for &x in s {
        add(x as u64);
    }
    add(b.len() as u64);
    (h ^ (h >> 32)) as u32
}
