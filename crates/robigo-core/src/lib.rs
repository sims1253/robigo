//! robigo-core: R objects as header'd heap objects behind one pointer (a
//! `SEXP`), on a non-moving mark-region heap with conservative stack
//! scanning (`docs/core.md`, goalpost G2).
//!
//! This crate knows memory.c-level R: object layout, allocation,
//! collection, CHARSXP interning and symbols. R semantics above that
//! (attributes, coercion, environments, evaluation) live in the crates
//! that use it (`robigo-capi` today, the runtime after G3).

#![allow(clippy::missing_safety_doc)]

mod gc;
mod heap;
pub mod unwind;

pub use gc::{Core, GcStats, ReleaseFn};

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
// HOST:      +16 handle, +24 release fn
// CHARSXP:   +8 hash u32 (no attributes), +16 length, +24 truelength, +32 bytes\0

pub const OFF_LEN: usize = 16;
pub const OFF_TRUELEN: usize = 24;
pub const OFF_DATA: usize = 32;
pub const NODE_SIZE: usize = 40;
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
    #[inline(always)]
    unsafe fn word(self, off: usize) -> *mut usize {
        unsafe { self.0.add(off) as *mut usize }
    }
    #[inline(always)]
    pub unsafe fn sexp_at(self, off: usize) -> Sexp {
        unsafe { Sexp(*(self.word(off) as *const *mut u8)) }
    }
    #[inline(always)]
    pub unsafe fn set_sexp_at(self, off: usize, v: Sexp) {
        unsafe { *(self.word(off) as *mut *mut u8) = v.0 }
    }

    // -- header
    #[inline(always)]
    pub unsafe fn typ(self) -> u8 {
        unsafe { *self.0 }
    }
    /// Change the type in place (GNU R's SET_TYPEOF).
    #[inline(always)]
    pub unsafe fn set_typ(self, t: u8) {
        unsafe { *self.0 = t }
    }
    #[inline(always)]
    pub unsafe fn flags(self) -> u8 {
        unsafe { *self.0.add(1) }
    }
    #[inline(always)]
    pub unsafe fn set_flags(self, f: u8) {
        unsafe { *self.0.add(1) = f }
    }
    #[inline(always)]
    pub unsafe fn has_flag(self, f: u8) -> bool {
        unsafe { self.flags() & f != 0 }
    }
    #[inline(always)]
    pub unsafe fn set_flag(self, f: u8, on: bool) {
        unsafe {
            let v = self.flags();
            self.set_flags(if on { v | f } else { v & !f });
        }
    }
    #[inline(always)]
    pub unsafe fn is_object(self) -> bool {
        unsafe { self.has_flag(flags::OBJECT) }
    }
    #[inline(always)]
    pub unsafe fn set_object(self, on: bool) {
        unsafe { self.set_flag(flags::OBJECT, on) }
    }
    /// GNU R's TRACKREFS.
    #[inline(always)]
    pub unsafe fn tracks_refs(self) -> bool {
        unsafe { self.typ() == CLOSXP || !self.has_flag(flags::NO_TRACKREFS) }
    }
    #[inline(always)]
    pub unsafe fn gp(self) -> u16 {
        unsafe { *(self.0.add(2) as *const u16) }
    }
    #[inline(always)]
    pub unsafe fn set_gp(self, v: u16) {
        unsafe { *(self.0.add(2) as *mut u16) = v }
    }
    #[inline(always)]
    pub unsafe fn refcnt(self) -> u16 {
        unsafe { *(self.0.add(4) as *const u16) }
    }
    #[inline(always)]
    pub unsafe fn set_refcnt(self, v: u16) {
        unsafe { *(self.0.add(4) as *mut u16) = v }
    }
    #[inline(always)]
    pub unsafe fn extra(self) -> u16 {
        unsafe { *(self.0.add(6) as *const u16) }
    }
    #[inline(always)]
    pub unsafe fn set_extra(self, v: u16) {
        unsafe { *(self.0.add(6) as *mut u16) = v }
    }
    /// The attribute pairlist (CHARSXPs have none: their slot holds the hash).
    #[inline(always)]
    pub unsafe fn attrib_raw(self) -> Sexp {
        unsafe { self.sexp_at(8) }
    }
    #[inline(always)]
    pub unsafe fn set_attrib_raw(self, v: Sexp) {
        unsafe { self.set_sexp_at(8, v) }
    }
    #[inline(always)]
    pub unsafe fn is_altrep(self) -> bool {
        unsafe { self.has_flag(flags::ALTREP) }
    }

    // -- vectors
    #[inline(always)]
    pub unsafe fn std_len(self) -> isize {
        unsafe { *(self.word(OFF_LEN) as *const isize) }
    }
    #[inline(always)]
    pub unsafe fn set_std_len(self, n: isize) {
        unsafe { *(self.word(OFF_LEN) as *mut isize) = n }
    }
    #[inline(always)]
    pub unsafe fn truelen(self) -> isize {
        unsafe { *(self.word(OFF_TRUELEN) as *const isize) }
    }
    #[inline(always)]
    pub unsafe fn set_truelen(self, n: isize) {
        unsafe { *(self.word(OFF_TRUELEN) as *mut isize) = n }
    }
    /// The payload of a non-ALTREP vector (inline, or EXTDATA's pointer).
    #[inline(always)]
    pub unsafe fn std_data(self) -> *mut u8 {
        unsafe {
            if self.has_flag(flags::EXTDATA) { *(self.word(OFF_DATA) as *const *mut u8) } else { self.0.add(OFF_DATA) }
        }
    }

    // -- nodes
    #[inline(always)]
    pub unsafe fn car(self) -> Sexp {
        unsafe { self.sexp_at(16) }
    }
    #[inline(always)]
    pub unsafe fn cdr(self) -> Sexp {
        unsafe { self.sexp_at(24) }
    }
    #[inline(always)]
    pub unsafe fn tag(self) -> Sexp {
        unsafe { self.sexp_at(32) }
    }
    #[inline(always)]
    pub unsafe fn set_car_raw(self, v: Sexp) {
        unsafe { self.set_sexp_at(16, v) }
    }
    #[inline(always)]
    pub unsafe fn set_cdr_raw(self, v: Sexp) {
        unsafe { self.set_sexp_at(24, v) }
    }
    #[inline(always)]
    pub unsafe fn set_tag_raw(self, v: Sexp) {
        unsafe { self.set_sexp_at(32, v) }
    }

    // -- CHARSXP
    #[inline(always)]
    pub unsafe fn char_hash(self) -> u32 {
        unsafe { *(self.0.add(8) as *const u32) }
    }
    /// The bytes of a CHARSXP (without the terminating NUL).
    #[inline(always)]
    pub unsafe fn char_bytes<'a>(self) -> &'a [u8] {
        unsafe { std::slice::from_raw_parts(self.0.add(OFF_DATA), self.std_len() as usize) }
    }

    // -- reference counts (copy-on-modify, GNU R's REFCNT protocol)
    #[inline(always)]
    pub unsafe fn incref(self) {
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
    #[inline(always)]
    pub unsafe fn decref(self) {
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
