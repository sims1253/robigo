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
pub mod slots;
mod trace;
pub mod unwind;

pub use gc::{
    AllocError, Core, GC_DUE, GcStats, HeapReport, LARGE, NeedGc, POISON_TYPE, PinReportTo, ReleaseFn, Safepoint,
    ScanSegment, ScanState, StatsMode, overshoot_bound,
};
pub use robigo_heap_derive::Trace;
pub use trace::{Trace, Visitor};

/// Which heap an object belongs to ([`Sexp::heap_id`], [`Core::id`]): the
/// owner checks of G3's runtime compare the two (docs/g3.md §2.3), so that
/// no isolate roots or stores another isolate's object.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HeapId(usize);

/// GNU R's SEXPTYPE numbers (Rinternals.h).
pub mod types {
    pub const NILSXP: u8 = 0;
    pub const SYMSXP: u8 = 1;
    pub const LISTSXP: u8 = 2;
    pub const CLOSXP: u8 = 3;
    pub const ENVSXP: u8 = 4;
    pub const PROMSXP: u8 = 5;
    pub const LANGSXP: u8 = 6;
    pub const SPECIALSXP: u8 = 7;
    pub const BUILTINSXP: u8 = 8;
    pub const CHARSXP: u8 = 9;
    pub const LGLSXP: u8 = 10;
    pub const INTSXP: u8 = 13;
    pub const REALSXP: u8 = 14;
    pub const CPLXSXP: u8 = 15;
    pub const STRSXP: u8 = 16;
    pub const DOTSXP: u8 = 17;
    pub const ANYSXP: u8 = 18;
    pub const VECSXP: u8 = 19;
    pub const EXPRSXP: u8 = 20;
    pub const BCODESXP: u8 = 21;
    pub const EXTPTRSXP: u8 = 22;
    pub const WEAKREFSXP: u8 = 23;
    pub const RAWSXP: u8 = 24;
    pub const OBJSXP: u8 = 25;
}
use types::*;

// The header's `extra` word (+6, `Sexp::extra`): bits 0 to 3 are GNU R's
// debug, trace, step and NOJIT bits (RDEBUG, RTRACE, RSTEP, NOJIT); bit 4
// is [`extra::SEEN_BY_C`], bit 5 [`extra::BYTECODE`]; a promise's
// `PROMISE_TAG` is bits 8-12; the others are free. A fresh allocation writes
// the whole header, so every bit starts clear.

/// robigo's bits of the header's `extra` word.
pub mod extra {
    /// The object was handed to C (a native routine's argument, an export's
    /// result, a word a C frame pinned): sticky, since C may keep the
    /// address where no collection looks. `ROBIGO_GC_MOVE=1` never moves
    /// such an object. Set it with [`crate::Sexp::mark_seen_by_c`], which
    /// keeps the other bits.
    pub const SEEN_BY_C: u16 = 1 << 4;
    /// GNU R holds this object's code as byte code (a BCODESXP), which
    /// robigo does not make in M1 (docs/g3.md §6.1, §9.1): on a closure,
    /// its BODY is byte code (installed with base, or compiled by GNU R's
    /// JIT); on a promise, its PRCODE is (compiled code made it). Set and
    /// read by robigo-rt's JIT port; `duplicate` copies it with the body.
    pub const BYTECODE: u16 = 1 << 5;
}

/// Header flag bits (byte +1).
pub mod flags {
    /// GNU R's `obj` bit: the object has a class attribute.
    pub const OBJECT: u8 = 0x01;
    pub const ALTREP: u8 = 0x02;
    /// The payload is elsewhere: a data pointer at +32, owned by +40/+48.
    pub const EXTDATA: u8 = 0x04;
    pub const IMMORTAL: u8 = 0x08;
    /// The barrier's state bit (council item 8): a store into this object
    /// must take the barrier's slow path ([`crate::Sexp::store`]), which
    /// logs the object in its heap's remembered set and clears the bit, so
    /// each object is logged once. Set today on every immortal object (made
    /// a root by its first store); a generational or incremental collector
    /// sets it on the objects it promotes (sticky marks, docs/core.md §11),
    /// so the inline fast path stays one test of this byte.
    pub const BARRIER: u8 = 0x10;
    /// Free. Was the second header mark color, retired when marks moved to
    /// the side bitmaps (docs/core.md §10). Kept for the barrier work: Ruby
    /// RGenGC's "write-barrier unprotected" bit for objects whose payload
    /// pointer C has (`DATAPTR` on a STRSXP or VECSXP: docs/core.md §11).
    pub const SPARE: u8 = 0x20;
    /// An ENVSXP laid out as a slot frame or an overflow block
    /// ([`crate::slots`]): its body is the slot map, not node fields.
    pub const SLOTS: u8 = 0x40;
    /// Do not maintain reference counts of the children: GNU R's `spare`
    /// bit (TRACKREFS(x) is `!spare`, and closures always track).
    pub const NO_TRACKREFS: u8 = 0x80;
}

/// Where a promise keeps GNU R's `PROMISE_TAG` (`IMMEDIATE_PROMISE_VALUES`,
/// `Defn.h:34-39`): bits 8-12 of the header's `extra` word, the type of the
/// unboxed value in PRVALUE (REALSXP, INTSXP or LGLSXP), 0 for none. Bits
/// 0-3 are GNU R's debug bits (closures), so the tag sits above them.
pub mod promise {
    pub const TAG_SHIFT: u16 = 8;
    pub const TAG_MASK: u16 = 0x1F << TAG_SHIFT;
}

/// GNU R's sxpinfo.gp bits that the core itself uses (Defn.h).
pub mod gp {
    pub const BYTES: u16 = 1 << 1;
    pub const LATIN1: u16 = 1 << 2;
    pub const UTF8: u16 = 1 << 3;
    pub const S4: u16 = 1 << 4;
    pub const CACHED: u16 = 1 << 5;
    pub const ASCII: u16 = 1 << 6;
    /// CHARSXP: TRUELENGTH holds `R_Newhashpjw` of the bytes (GNU R's
    /// HASHASH, set on every symbol's PRINTNAME by install).
    pub const HASHASH: u16 = 1;
    pub const GROWABLE: u16 = 1 << 5;
    /// weak references: ready to finalize (memory.c READY_TO_FINALIZE_MASK)
    pub const READY_TO_FINALIZE: u16 = 1;
    /// weak references: run the finalizer at exit (FINALIZE_ON_EXIT_MASK)
    pub const FINALIZE_ON_EXIT: u16 = 2;
}

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
/// the fixed layout in the table below `NA_REAL`, and a store through one of
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

// ---- layout ---------------------------------------------------------------
// +0 type u8, +1 flags u8, +2 gp u16, +4 refcnt u16, +6 extra u16, +8 attrib
// vectors:   +16 length i64, +24 truelength i64, +32 payload
// EXTDATA:   +32 data pointer, +40 owner word, +48 release fn
// nodes:     +16 car, +24 cdr, +32 tag, +40 aux (the 4th word, traced)
//            ENVSXP fixes only ENCLOS (+24); +16, +32 and +40 are the frame's
// SYMSXP:    +16 PRINTNAME, +24 SYMVALUE, +32 INTERNAL, +40 the symbol's id
//            (low 32 bits) and R_Newhashpjw of its name (high 32): not an object
// ALTREP:    +16 data1, +24 data2, +32 class, +40 aux (nil)
// CHARSXP:   +8 hash u32 (no attributes), +16 length, +24 truelength, +32 bytes\0

pub const OFF_LEN: usize = 16;
pub const OFF_TRUELEN: usize = 24;
pub const OFF_DATA: usize = 32;
/// The 4th node word (council item 20). Every node-shaped object (made by
/// [`Core::alloc_node`]) has it, R_NilValue when made, and every collection
/// traces it. No C API reaches it: it is the runtime's (G3), reserved for
/// the environment's slot frame, a closure's compiled-code cache and a
/// call's call-site cache (docs/core.md §2).
pub const OFF_AUX: usize = 40;
/// A symbol's 4th word: its id (low 32 bits) and GNU R's `R_Newhashpjw` of
/// its name (high 32 bits), set when the symbol is made and never changed.
/// Not an object: the collector does not trace it. The id is dense, in the
/// order the heap made its symbols (`R_UnboundValue` 0, `R_MissingArg` 1,
/// then installs): the runtime's per-isolate binding tables are indexed by
/// it (docs/g3.md §5.3; M2's shared symbol arena computes it instead).
pub const OFF_SYMID: usize = OFF_AUX;
/// The one fixed field of an environment's body: ENCLOS. FRAME and HASHTAB
/// are shims that materialize from whatever the frame words contain
/// ([`env`]), so a frame representation is free to change.
pub const OFF_ENCLOS: usize = 24;
/// A node: header, car, cdr, tag and the 4th word: the 48-byte class,
/// with nothing to spare (GNU R's cons cell is 56 bytes).
pub const NODE_SIZE: usize = 48;
/// An EXTDATA vector ([`Core::alloc_extdata`]): the standard header and
/// lengths, then its payload pointer where a standard vector's payload
/// starts, its owner word and its release function, read and written only
/// through these offsets.
pub const EXTDATA_SIZE: usize = 56;
/// EXTDATA's payload pointer.
pub const OFF_EXTDATA_PTR: usize = OFF_DATA;
/// EXTDATA's owner word, the argument of its release function.
pub const OFF_EXTDATA_OWNER: usize = 40;
/// EXTDATA's release function, an `Option<ReleaseFn>`: null is `None`,
/// which C code that disarms the call writes.
pub const OFF_EXTDATA_RELEASE: usize = 48;
const _: () = assert!(
    OFF_EXTDATA_PTR + 8 == OFF_EXTDATA_OWNER
        && OFF_EXTDATA_OWNER + 8 == OFF_EXTDATA_RELEASE
        && OFF_EXTDATA_RELEASE + 8 == EXTDATA_SIZE
);

/// Bytes per element of a vector type (0: not a vector).
#[inline]
pub fn elt_size(t: u8) -> usize {
    match t {
        LGLSXP | INTSXP => 4,
        REALSXP => 8,
        CPLXSXP => 16,
        STRSXP | VECSXP | EXPRSXP | WEAKREFSXP => 8,
        RAWSXP | CHARSXP => 1,
        _ => 0,
    }
}

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
    unsafe fn word(self, off: usize) -> *mut usize {
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

    // -- header
    /// The SEXPTYPE.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn typ(self) -> u8 {
        // SAFETY: the first byte of every live object's header.
        unsafe { *self.0 }
    }
    /// Change the type in place (GNU R's SET_TYPEOF).
    ///
    /// # Safety
    /// `self` must be live, and its allocation and contents must be valid
    /// for `t`: the collector and every accessor read the body by type. So
    /// a change stays within one body shape (LISTSXP to LANGSXP, one atomic
    /// vector type to another of the same element size or a smaller one). A
    /// vector made a node would have its length traced as a car; a CHARSXP
    /// made anything else would leave the cache containing it.
    #[inline(always)]
    pub unsafe fn set_typ(self, t: u8) {
        // SAFETY: the first header byte of a live object.
        unsafe { *self.0 = t }
    }
    /// The header flags (the `flags` module).
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn flags(self) -> u8 {
        // SAFETY: header byte +1 of a live object.
        unsafe { *self.0.add(1) }
    }
    /// Overwrite the header flags.
    ///
    /// # Safety
    /// `self` must be live, and the new flags must describe its body:
    /// ALTREP and EXTDATA change how it is read, by accessors and by
    /// the collector. IMMORTAL goes through [`Core::make_immortal`], which
    /// also counts it for the sweep.
    #[inline(always)]
    pub unsafe fn set_flags(self, f: u8) {
        // SAFETY: header byte +1 of a live object.
        unsafe { *self.0.add(1) = f }
    }
    /// Whether any of the bits `f` is set.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn has_flag(self, f: u8) -> bool {
        // SAFETY: `self` is live, all `flags` needs.
        unsafe { self.flags() & f != 0 }
    }
    /// Set or clear the bits `f`.
    ///
    /// # Safety
    /// As for [`Sexp::set_flags`].
    #[inline(always)]
    pub unsafe fn set_flag(self, f: u8, on: bool) {
        // SAFETY: `self` is live, and the caller vouches for the new flags.
        unsafe {
            let v = self.flags();
            self.set_flags(if on { v | f } else { v & !f });
        }
    }
    /// GNU R's OBJECT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn is_object(self) -> bool {
        // SAFETY: `self` is live, all `has_flag` needs.
        unsafe { self.has_flag(flags::OBJECT) }
    }
    /// GNU R's SET_OBJECT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn set_object(self, on: bool) {
        // SAFETY: `self` is live, and OBJECT does not change how it is read.
        unsafe { self.set_flag(flags::OBJECT, on) }
    }
    /// GNU R's TRACKREFS.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn tracks_refs(self) -> bool {
        // SAFETY: `self` is live, all these header reads need.
        unsafe { self.typ() == CLOSXP || !self.has_flag(flags::NO_TRACKREFS) }
    }
    /// GNU R's LEVELS.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn gp(self) -> u16 {
        // SAFETY: header bytes +2..+4 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(2) as *const u16) }
    }
    /// GNU R's SETLEVELS.
    ///
    /// # Safety
    /// `self` must be live. On a CHARSXP the bits are part of the cache's
    /// bookkeeping: CACHED must stay true of the object (every cached
    /// CHARSXP is in the table, and the prune counts on it), and the encoding
    /// bits must stay the ones it was interned under.
    #[inline(always)]
    pub unsafe fn set_gp(self, v: u16) {
        // SAFETY: header bytes +2..+4 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(2) as *mut u16) = v }
    }
    /// GNU R's REFCNT.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn refcnt(self) -> u16 {
        // SAFETY: header bytes +4..+6 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(4) as *const u16) }
    }
    /// Overwrite the count.
    ///
    /// # Safety
    /// `self` must be live. Too low a count is a logic error with teeth: code
    /// that trusts `MAYBE_SHARED` then modifies a shared object in place.
    #[inline(always)]
    pub unsafe fn set_refcnt(self, v: u16) {
        // SAFETY: header bytes +4..+6 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(4) as *mut u16) = v }
    }
    /// The debug, trace and step bits.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn extra(self) -> u16 {
        // SAFETY: header bytes +6..+8 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(6) as *const u16) }
    }
    /// Overwrite the debug, trace and step bits.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn set_extra(self, v: u16) {
        // SAFETY: header bytes +6..+8 of a live, 16-byte aligned object.
        unsafe { *(self.0.add(6) as *mut u16) = v }
    }
    /// GNU R's `PROMISE_TAG` of a promise (`IMMEDIATE_PROMISE_VALUES`,
    /// `Defn.h:34-39, 1268-1275`): 0, or the type (REALSXP, INTSXP, LGLSXP)
    /// of the unboxed value whose raw bits PRVALUE (+16) holds. A tagged
    /// promise is evaluated, and the collector does not trace +16
    /// ([`Sexp::trace_fields`]).
    ///
    /// # Safety
    /// `self` must be a live PROMSXP.
    #[inline(always)]
    pub unsafe fn promise_tag(self) -> u8 {
        // SAFETY: the caller's contract.
        unsafe { ((self.extra() & promise::TAG_MASK) >> promise::TAG_SHIFT) as u8 }
    }
    /// Set `PROMISE_TAG`. The caller writes +16 to match in the same
    /// interval: raw bits for a tag, an object (its count dropped from the
    /// raw bits it replaces: none) for 0.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP; `t` is 0, REALSXP, INTSXP or LGLSXP;
    /// when the tag goes from 0 to a type, +16 holds no object whose count
    /// is still needed (the caller dropped it).
    #[inline(always)]
    pub unsafe fn set_promise_tag(self, t: u8) {
        debug_assert!(matches!(t, 0 | REALSXP | INTSXP | LGLSXP));
        // SAFETY: the caller's contract.
        unsafe {
            let e = self.extra() & !promise::TAG_MASK;
            self.set_extra(e | ((t as u16) << promise::TAG_SHIFT));
        }
    }
    /// The raw bits of PRVALUE (+16): a tagged promise's unboxed value.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP.
    #[inline(always)]
    pub unsafe fn promise_bits(self) -> u64 {
        // SAFETY: +16 of a node.
        unsafe { *(self.word(16) as *const u64) }
    }
    /// Make PRVALUE (+16) the object `v` (or R_UnboundValue), whatever it
    /// held: GNU R's `SET_PRVALUE` on a promise, with `CLEAR_BNDCELL_TAG`
    /// first when the value was unboxed (`PROMISE_TAG`). Raw bits are no
    /// object, so no count is dropped for them (and they are never read as
    /// a pointer); an object it held loses its count as any store's does.
    /// `v` is counted and the store takes the barrier.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP and `v` a live object of its heap.
    // R: memory.c:SET_PRVALUE, Defn.h:CLEAR_BNDCELL_TAG
    #[inline]
    pub unsafe fn set_promise_value(self, v: Sexp) {
        // SAFETY: the caller's contract; +16 is a node field.
        unsafe {
            if self.promise_tag() != 0 {
                self.set_promise_tag(0);
                // a null word: the store below drops no count for it
                self.set_sexp_at(16, Sexp::NULL);
            }
            self.store(self.field(16), v);
        }
    }
    /// Write raw bits into PRVALUE (+16), touching no count: the store of
    /// GNU R's `SET_BNDCELL_DVAL`/`IVAL`/`LVAL` on a promise.
    ///
    /// # Safety
    /// `self` must be a live PROMSXP whose tag is (or is being set to) a
    /// type, and whose +16 held no counted object (or the caller dropped it).
    #[inline(always)]
    pub unsafe fn set_promise_bits(self, bits: u64) {
        // SAFETY: the caller's contract.
        unsafe { *(self.word(16) as *mut u64) = bits }
    }
    /// Mark the object as handed to C ([`extra::SEEN_BY_C`]): from now on a
    /// moving collection leaves it where it is.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn mark_seen_by_c(self) {
        // SAFETY: the caller's contract.
        unsafe { self.set_extra(self.extra() | extra::SEEN_BY_C) }
    }
    /// Whether the object was ever handed to C ([`extra::SEEN_BY_C`]).
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn seen_by_c(self) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.extra() & extra::SEEN_BY_C != 0 }
    }
    /// The attribute pairlist (CHARSXPs have none: their slot contains the hash).
    ///
    /// # Safety
    /// `self` must be live. On a CHARSXP the result is the hash, not an
    /// object, and must not be dereferenced or stored as one.
    #[inline(always)]
    pub unsafe fn attrib_raw(self) -> Sexp {
        // SAFETY: +8 is part of every object's 16-byte header.
        unsafe { self.sexp_at(8) }
    }
    /// Store the attribute pairlist, with no reference count and no barrier.
    ///
    /// # Safety
    /// `self` must be live and not a CHARSXP: this would overwrite its hash,
    /// and the cache could no longer find it or place it when it grows. `v`
    /// must be live (a pairlist or R_NilValue): every collection traces
    /// it. On an immortal `self` the store also needs the barrier
    /// ([`Sexp::store`]), or nothing keeps `v` alive.
    #[inline(always)]
    pub(crate) unsafe fn set_attrib_raw(self, v: Sexp) {
        // SAFETY: +8 is part of every object's 16-byte header; what `v` must
        // be is the caller's contract.
        unsafe { self.set_sexp_at(8, v) }
    }
    /// GNU R's ALTREP.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn is_altrep(self) -> bool {
        // SAFETY: `self` is live, all `has_flag` needs.
        unsafe { self.has_flag(flags::ALTREP) }
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

    // -- vectors
    /// The length of a standard vector.
    ///
    /// # Safety
    /// `self` must be a live vector or CHARSXP that is not ALTREP. On an
    /// ALTREP object +16 is `data1`, a pointer that reads as a nonsense
    /// length.
    #[inline(always)]
    pub unsafe fn std_len(self) -> isize {
        // SAFETY: the caller's contract puts a length at OFF_LEN.
        unsafe { *(self.word(OFF_LEN) as *const isize) }
    }
    /// Store the length of a standard vector.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`], and `n` must not exceed the capacity of the
    /// payload: the collector and every reader trust it, and a STRSXP or
    /// VECSXP has its first `n` elements traced as objects.
    #[inline(always)]
    pub unsafe fn set_std_len(self, n: isize) {
        // SAFETY: the caller's contract puts a length at OFF_LEN.
        unsafe { *(self.word(OFF_LEN) as *mut isize) = n }
    }
    /// GNU R's TRUELENGTH.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`]: +24 is `data2` or a node field otherwise.
    #[inline(always)]
    pub unsafe fn truelen(self) -> isize {
        // SAFETY: the caller's contract puts a true length at OFF_TRUELEN.
        unsafe { *(self.word(OFF_TRUELEN) as *const isize) }
    }
    /// GNU R's SET_TRUELENGTH.
    ///
    /// # Safety
    /// As for [`Sexp::std_len`].
    #[inline(always)]
    pub unsafe fn set_truelen(self, n: isize) {
        // SAFETY: the caller's contract puts a true length at OFF_TRUELEN.
        unsafe { *(self.word(OFF_TRUELEN) as *mut isize) = n }
    }
    /// The payload of a non-ALTREP vector (inline, or EXTDATA's pointer).
    ///
    /// # Safety
    /// As for [`Sexp::std_len`]: an ALTREP object's +32 is its class, and a
    /// node has no payload. The pointer stays valid while the
    /// object is live (objects do not move); an EXTDATA pointer is as good
    /// as its owner's promise to [`Core::alloc_extdata`].
    #[inline(always)]
    pub unsafe fn std_data(self) -> *mut u8 {
        // SAFETY: the caller's contract makes this a standard vector, whose
        // payload starts at OFF_DATA or, for EXTDATA, is stored there.
        unsafe {
            if self.has_flag(flags::EXTDATA) {
                *(self.word(OFF_EXTDATA_PTR) as *const *mut u8)
            } else {
                self.0.add(OFF_DATA)
            }
        }
    }

    // -- nodes
    /// GNU R's CAR, and the fields that share its slot.
    ///
    /// # Safety
    /// `self` must be a live object with node fields: a node or an ALTREP
    /// object. Not a vector, where +16 is the length. The car of
    /// an EXTPTRSXP or a primitive is not an object.
    #[inline(always)]
    pub unsafe fn car(self) -> Sexp {
        // SAFETY: the caller's contract puts a field at +16.
        unsafe { self.sexp_at(16) }
    }
    /// GNU R's CDR, and the fields that share its slot.
    ///
    /// # Safety
    /// As for [`Sexp::car`]: on a vector +24 is the true length.
    #[inline(always)]
    pub unsafe fn cdr(self) -> Sexp {
        // SAFETY: the caller's contract puts a field at +24.
        unsafe { self.sexp_at(24) }
    }
    /// GNU R's TAG, and the fields that share its slot.
    ///
    /// # Safety
    /// As for [`Sexp::car`]: on a vector +32 is payload.
    #[inline(always)]
    pub unsafe fn tag(self) -> Sexp {
        // SAFETY: the caller's contract puts a field at +32.
        unsafe { self.sexp_at(32) }
    }
    /// Store the car, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::car`]. `v` must be live unless `self`'s car is not an
    /// object (EXTPTRSXP, BUILTINSXP, SPECIALSXP), and a live `v` needs the
    /// barrier ([`Sexp::store`]) if `self` is immortal, or nothing keeps it
    /// alive. Reference counts are the caller's to keep.
    #[inline(always)]
    pub(crate) unsafe fn set_car_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +16 and vouches for `v`.
        unsafe { self.set_sexp_at(16, v) }
    }
    /// Store the cdr, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub(crate) unsafe fn set_cdr_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +24 and vouches for `v`.
        unsafe { self.set_sexp_at(24, v) }
    }
    /// Store the tag, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub(crate) unsafe fn set_tag_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +32 and vouches for `v`.
        unsafe { self.set_sexp_at(32, v) }
    }

    /// The 4th node word ([`OFF_AUX`]).
    ///
    /// # Safety
    /// `self` must be a live node made by [`Core::alloc_node`] (not a vector).
    #[inline(always)]
    pub unsafe fn aux(self) -> Sexp {
        // SAFETY: the caller's contract puts the 4th word at +40.
        unsafe { self.sexp_at(OFF_AUX) }
    }

    /// A symbol's id ([`OFF_SYMID`]): dense, in the order this heap made
    /// its symbols.
    ///
    /// # Safety
    /// `self` must be a live SYMSXP.
    #[inline(always)]
    pub unsafe fn sym_id(self) -> u32 {
        // SAFETY: the caller's contract: a symbol's 4th word holds its id in
        // the low half (little or big endian, read as a whole word).
        unsafe { *(self.word(OFF_SYMID) as *const u64) as u32 }
    }
    /// GNU R's `R_Newhashpjw` of a symbol's name ([`sym_hash`]), stored
    /// when the symbol was made ([`OFF_SYMID`]).
    ///
    /// # Safety
    /// `self` must be a live SYMSXP.
    #[inline(always)]
    pub unsafe fn sym_hash(self) -> u32 {
        // SAFETY: as for `sym_id`: the high half of the same word.
        unsafe { (*(self.word(OFF_SYMID) as *const u64) >> 32) as u32 }
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
    /// The field at byte offset `off`, as [`Sexp::store`] takes it.
    ///
    /// # Safety
    /// `self` must be live and `off` inside its allocation.
    #[inline(always)]
    unsafe fn field(self, off: usize) -> *mut Sexp {
        // SAFETY: the caller's contract.
        unsafe { self.word(off) as *mut Sexp }
    }
    /// GNU R's SET_ATTRIB store (without its checks).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is not a CHARSXP and `v` is a
    /// pairlist or R_NilValue.
    #[inline(always)]
    pub unsafe fn set_attrib(self, v: Sexp) {
        // SAFETY: +8 is the attribute slot of every non-CHARSXP header.
        unsafe { self.store(self.field(8), v) }
    }
    /// GNU R's SETCAR store (also SET_PRVALUE, SET_FORMALS, ALTREP data1).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields and its car is an
    /// object (not an EXTPTRSXP, BUILTINSXP or SPECIALSXP).
    #[inline(always)]
    pub unsafe fn set_car(self, v: Sexp) {
        // SAFETY: the caller's contract puts a traced field at +16.
        unsafe { self.store(self.field(16), v) }
    }
    /// GNU R's SETCDR store.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields.
    #[inline(always)]
    pub unsafe fn set_cdr(self, v: Sexp) {
        // SAFETY: the caller's contract puts a traced field at +24.
        unsafe { self.store(self.field(24), v) }
    }
    /// GNU R's SET_TAG store.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` has node fields.
    #[inline(always)]
    pub unsafe fn set_tag(self, v: Sexp) {
        // SAFETY: the caller's contract puts a traced field at +32.
        unsafe { self.store(self.field(32), v) }
    }
    /// Store the 4th node word.
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is as for [`Sexp::aux`].
    #[inline(always)]
    pub unsafe fn set_aux(self, v: Sexp) {
        // SAFETY: the caller's contract puts the traced 4th word at +40.
        unsafe { self.store(self.field(OFF_AUX), v) }
    }
    /// GNU R's SET_VECTOR_ELT / SET_STRING_ELT store (without the checks).
    ///
    /// # Safety
    /// As for [`Sexp::store`]; `self` is a standard (not ALTREP) STRSXP,
    /// VECSXP, EXPRSXP or WEAKREFSXP and `i` is below its length.
    #[inline(always)]
    pub unsafe fn set_elt(self, i: usize, v: Sexp) {
        // SAFETY: the caller's contract: element `i` of a pointer payload.
        unsafe { self.store((self.std_data() as *mut Sexp).add(i), v) }
    }
    /// Store an external pointer's address (its car, which is not an
    /// object): no count and no barrier, as GNU R's R_SetExternalPtrAddr.
    ///
    /// # Safety
    /// `self` must be a live EXTPTRSXP.
    #[inline(always)]
    pub unsafe fn set_extptr_addr(self, p: *mut u8) {
        // SAFETY: an EXTPTRSXP's car at +16 is an untraced word.
        unsafe { self.set_sexp_at(16, Sexp(p)) }
    }

    // -- CHARSXP
    /// The hash stored in a CHARSXP's header (the attribute slot).
    ///
    /// # Safety
    /// `self` must be a live CHARSXP. On anything else this reads the low
    /// half of the attribute pointer.
    #[inline(always)]
    pub unsafe fn char_hash(self) -> u32 {
        // SAFETY: +8 is in the header of every object, and 8-byte aligned.
        unsafe { *(self.0.add(8) as *const u32) }
    }
    /// The bytes of a CHARSXP (without the terminating NUL).
    ///
    /// # Safety
    /// `self` must be a live CHARSXP that stays live for `'a`. Nothing ties
    /// `'a` to the object: the caller picks it, and the slice dangles as
    /// soon as the CHARSXP is swept. If it is not rooted, that is the next
    /// allocation. The bytes must not change for `'a` either.
    #[inline(always)]
    pub unsafe fn char_bytes<'a>(self) -> &'a [u8] {
        // SAFETY: a CHARSXP has `std_len` initialized bytes at OFF_DATA, and
        // the caller keeps them there for `'a`.
        unsafe { std::slice::from_raw_parts(self.0.add(OFF_DATA), self.std_len() as usize) }
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
/// offset ([`OFF_ENCLOS`]). FRAME and HASHTAB are shims over the frame
/// words (+16, +32, +40), so the runtime is free to change what those contain
/// (G3's slot frames): then `frame` builds the pairlist of bindings that C
/// expects, and `hashtab` the hashed frame, from it. No package imports
/// FRAME or HASHTAB (`tests/capi/symbols.tsv`); C reaches an environment's
/// bindings through `Rf_findVarInFrame` & co.
///
/// A hashed environment's frame words contain GNU R's representation: +16
/// the frame (R_NilValue), +32 the hash table (a VECSXP of chains), +40 the
/// runtime's version box or R_NilValue. An unhashed one is a slot frame
/// ([`crate::flags::SLOTS`], [`crate::slots`]): `frame` and `hashtab` do not
/// apply to it (the runtime builds FRAME's snapshot from the slots), and
/// debug builds assert so.
///
/// Every function here takes a live, node-shaped ENVSXP (from
/// [`Core::alloc_node`]) and, for a store, a live value.
pub mod env {
    use crate::{OFF_ENCLOS, Sexp};

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
        unsafe { x.set_cdr(v) }
    }
    /// GNU R's FRAME: the bindings as a pairlist of tagged cells.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    // R: memory.c:FRAME
    pub unsafe fn frame(x: Sexp) -> Sexp {
        // SAFETY: the frame representation above: the pairlist at +16.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: FRAME of a slot frame (its +16 is the overflow)");
            x.car()
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
            x.set_car(v)
        }
    }
    /// GNU R's HASHTAB: the hashed frame, or R_NilValue for an unhashed one.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    // R: memory.c:HASHTAB
    pub unsafe fn hashtab(x: Sexp) -> Sexp {
        // SAFETY: the frame representation above: the table at +32.
        unsafe {
            debug_assert!(!x.has_flag(crate::flags::SLOTS), "robigo: HASHTAB of a slot frame (its +32 is the shape)");
            x.tag()
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
            x.set_tag(v)
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
