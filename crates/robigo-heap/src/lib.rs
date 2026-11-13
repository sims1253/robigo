//! robigo-heap: R objects as header'd heap objects behind one pointer (a
//! `SEXP`), on a non-moving mark-region heap. The memory manager of G3's
//! runtime (`docs/g3.md`, the appendix's heap rows).
//!
//! The collection protocol (docs/g3.md §3, H2): allocation never collects;
//! it sets [`GC_DUE`] in a poll word, or refuses with [`NeedGc`] what only
//! a collection could serve. Collections run where the embedder asks, with
//! its roots as a [`Trace`] value: at a safepoint ([`Core::safepoint`]),
//! before an allocation that may collect ([`Core::alloc_rooted`],
//! [`Core::make_room`]), or explicitly ([`Core::collect`]). Their order is
//! precise roots, weak references and finalizers, C segments, the CHARSXP
//! cache, the sweep; the conservative scan reads only C segments, the
//! frames of package code between a native entry and the export it called,
//! never a Rust frame. `ROBIGO_GC_STATS` reports collections per safepoint
//! kind, E4's overshoot and allocations by type ([`StatsMode`]).
//!
//! A fork of `crates/robigo-core` at main `25bad70835a1c9bd19c70ffb26b1061987b7873a`
//! (2026-11-13; robigo-core last changed in `be7fcd3b`), with the appendix's
//! *removed* row deleted: everything that existed only for the old
//! runtime's native bridge (the list is in `docs/ledger/g3.md`, H1). The
//! experiment A switches (`ROBIGO_GC_PIN_ONLY`, `ROBIGO_CALL65_PLAIN`) are
//! kept as they were. robigo-core stays as it was for the old
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
mod trace;
pub mod unwind;

pub use gc::{
    AllocError, Core, GC_DUE, GcStats, HeapReport, LARGE, NeedGc, ReleaseFn, Safepoint, ScanSegment, StatsMode,
    overshoot_bound,
};
pub use robigo_heap_derive::Trace;
pub use trace::{Trace, Visitor};

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
// debug, trace, step and NOJIT bits (RDEBUG, RTRACE, RSTEP, NOJIT); the
// others are free. A fresh allocation writes the whole header, so every bit
// starts clear.

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
    // 0x40 is free.
    /// Do not maintain reference counts of the children: GNU R's `spare`
    /// bit (TRACKREFS(x) is `!spare`, and closures always track).
    pub const NO_TRACKREFS: u8 = 0x80;
}

/// GNU R's sxpinfo.gp bits that the core itself uses (Defn.h).
pub mod gp {
    pub const BYTES: u16 = 1 << 1;
    pub const LATIN1: u16 = 1 << 2;
    pub const UTF8: u16 = 1 << 3;
    pub const S4: u16 = 1 << 4;
    pub const CACHED: u16 = 1 << 5;
    pub const ASCII: u16 = 1 << 6;
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
/// The one fixed field of an environment's body: ENCLOS. FRAME and HASHTAB
/// are shims that materialize from whatever the frame words contain
/// ([`env`]), so a frame representation is free to change.
pub const OFF_ENCLOS: usize = 24;
/// A node: header, car, cdr, tag and the 4th word: the 48-byte class,
/// with nothing to spare (GNU R's cons cell is 56 bytes).
pub const NODE_SIZE: usize = 48;
pub const EXTDATA_SIZE: usize = 56;

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
            if self.has_flag(flags::EXTDATA) { *(self.word(OFF_DATA) as *const *mut u8) } else { self.0.add(OFF_DATA) }
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
/// The frame words today contain GNU R's representation: +16 the frame (a
/// pairlist of bindings), +32 the hash table (a VECSXP of chains, or
/// R_NilValue), +40 R_NilValue. Materializing is reading it.
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
        unsafe { x.car() }
    }
    /// GNU R's SET_FRAME.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP, `v` a live pairlist or R_NilValue.
    // R: memory.c:SET_FRAME
    pub unsafe fn set_frame(x: Sexp, v: Sexp) {
        // SAFETY: as above.
        unsafe { x.set_car(v) }
    }
    /// GNU R's HASHTAB: the hashed frame, or R_NilValue for an unhashed one.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP.
    // R: memory.c:HASHTAB
    pub unsafe fn hashtab(x: Sexp) -> Sexp {
        // SAFETY: the frame representation above: the table at +32.
        unsafe { x.tag() }
    }
    /// GNU R's SET_HASHTAB.
    ///
    /// # Safety
    /// `x` is a live core ENVSXP, `v` a live VECSXP or R_NilValue.
    // R: memory.c:SET_HASHTAB
    pub unsafe fn set_hashtab(x: Sexp, v: Sexp) {
        // SAFETY: as above.
        unsafe { x.set_tag(v) }
    }
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
