//! robigo-core: R objects as header'd heap objects behind one pointer (a
//! `SEXP`), on a non-moving mark-region heap with conservative stack
//! scanning (`docs/core.md`, goalpost G2).
//!
//! This crate knows memory.c-level R: object layout, allocation,
//! collection, CHARSXP interning and symbols. R semantics above that
//! (attributes, coercion, environments, evaluation) live in the crates
//! that use it (`robigo-capi` today, the runtime after G3).

mod chars;
mod gc;
mod heap;
mod heapsize;
pub mod unwind;

pub use gc::{Core, GcStats, HeapReport, ReleaseFn};

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

/// Bits of the header's `extra` word (+6) besides GNU R's debug, trace
/// and step bits (RDEBUG 1, RTRACE 2, NOJIT 4).
/// Bits of the header's `extra` word (+6). Bits 0 to 3 are GNU R's debug,
/// trace, step and NOJIT bits (RDEBUG, RTRACE, RSTEP, NOJIT); the others
/// are robigo's. A fresh allocation writes the whole header, so every bit
/// starts clear.
///
/// | bit | name | set by |
/// |---|---|---|
/// | 8 | [`TWIN`] | the bridge's twins (cells of a runtime value's long-lived core copy) |
/// | 14 | [`HOST_OWNED`] | the bridge's zero-copy atomic vectors (EXTDATA over a runtime vector's buffer) |
/// | 15 | [`WATCHED`] | the bridge's proxy attribute cells (#386) |
pub mod extra {
    /// A cell of a twin (docs/core.md §6): the header, spine or attribute
    /// cell of the core object the bridge keeps for a runtime value. The C
    /// API's setters (`SETCAR`, `SET_VECTOR_ELT`, `SET_ATTRIB`, a writable
    /// `DATAPTR`, ...) report writes to it, which the bridge writes back
    /// into the runtime value.
    pub const TWIN: u16 = 1 << 8;
    /// An EXTDATA vector whose owner word belongs to the host bridge: a
    /// runtime atomic vector shared with C zero-copy (docs/core.md §6).
    pub const HOST_OWNED: u16 = 1 << 14;
    /// The cell is an attribute cell of a host proxy that the bridge
    /// recorded: SETCAR/SETCDR/SET_TAG tell the bridge when C writes it.
    pub const WATCHED: u16 = 1 << 15;
}

/// Header flag bits (byte +1).
pub mod flags {
    /// GNU R's `obj` bit: the object has a class attribute.
    pub const OBJECT: u8 = 0x01;
    pub const ALTREP: u8 = 0x02;
    /// The payload is elsewhere: a data pointer at +32, owned by +40/+48.
    pub const EXTDATA: u8 = 0x04;
    pub const IMMORTAL: u8 = 0x08;
    pub const MARK_A: u8 = 0x10;
    pub const MARK_B: u8 = 0x20;
    /// A proxy for an object of the host runtime (handle at +16).
    pub const HOST: u8 = 0x40;
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

/// One R object: a pointer to its header. The same value C code holds as
/// `SEXP`.
///
/// A `Sexp` is a plain address and says nothing about what it points at, so
/// every accessor that reads through it is `unsafe`. Their contracts share
/// one word: a *live* object is one a [`Core`] allocated and has not swept.
/// Liveness is not a property of the `Sexp`: an object stays live only while
/// the rooting rule (docs/core.md section 3) holds for it, so an unrooted one
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
// nodes:     +16 car, +24 cdr, +32 tag
// ALTREP:    +16 data1, +24 data2, +32 class
// HOST:      +16/+24/+32 node fields (nil), +40 handle, +48 release fn
// CHARSXP:   +8 hash u32 (no attributes), +16 length, +24 truelength, +32 bytes\0

pub const OFF_LEN: usize = 16;
pub const OFF_TRUELEN: usize = 24;
pub const OFF_DATA: usize = 32;
pub const NODE_SIZE: usize = 40;
pub const EXTDATA_SIZE: usize = 56;
pub const HOST_SIZE: usize = 56;

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
    /// [`Core::write_barrier`] or `v` is not kept alive.
    #[inline(always)]
    pub unsafe fn set_sexp_at(self, off: usize, v: Sexp) {
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
    /// made anything else would leave the cache holding it.
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
    /// ALTREP, EXTDATA and HOST change how it is read, by accessors and by
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
    /// Is this a cell of a twin ([`extra::TWIN`])?
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn is_twin_cell(self) -> bool {
        // SAFETY: the caller's contract.
        unsafe { self.extra() & extra::TWIN != 0 }
    }
    /// Set or clear [`extra::TWIN`], keeping the other bits.
    ///
    /// # Safety
    /// `self` must be live.
    #[inline(always)]
    pub unsafe fn set_twin_cell(self, on: bool) {
        // SAFETY: the caller's contract.
        unsafe {
            let e = self.extra();
            self.set_extra(if on { e | extra::TWIN } else { e & !extra::TWIN });
        }
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
    /// The attribute pairlist (CHARSXPs have none: their slot holds the hash).
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
    /// it. On an immortal `self` the store also needs
    /// [`Core::write_barrier`], or nothing keeps `v` alive.
    #[inline(always)]
    pub unsafe fn set_attrib_raw(self, v: Sexp) {
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
    /// `self` must be a live vector or CHARSXP that is not ALTREP and not
    /// HOST. On those +16 is `data1` or a node field, a pointer that reads
    /// as a nonsense length.
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
    /// HOST or node object has no payload. The pointer stays valid while the
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
    /// `self` must be a live object with node fields: a node, a HOST proxy
    /// or an ALTREP object. Not a vector, where +16 is the length. The car of
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
    /// object (EXTPTRSXP, BUILTINSXP, SPECIALSXP), and a live `v` needs a
    /// [`Core::write_barrier`] call if `self` is immortal, or nothing keeps
    /// it alive. Reference counts are the caller's to keep (`fix_refcnt`).
    #[inline(always)]
    pub unsafe fn set_car_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +16 and vouches for `v`.
        unsafe { self.set_sexp_at(16, v) }
    }
    /// Store the cdr, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub unsafe fn set_cdr_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +24 and vouches for `v`.
        unsafe { self.set_sexp_at(24, v) }
    }
    /// Store the tag, with no reference count and no barrier.
    ///
    /// # Safety
    /// As for [`Sexp::set_car_raw`], except that `v` must always be live.
    #[inline(always)]
    pub unsafe fn set_tag_raw(self, v: Sexp) {
        // SAFETY: the caller's contract puts a field at +32 and vouches for `v`.
        unsafe { self.set_sexp_at(32, v) }
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
