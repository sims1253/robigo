//! Code objects: what the compiler makes and the VM runs (docs/g3.md §6.1,
//! §9.2), as ordinary heap objects, so a collection frees them with their
//! last user and moves them like any object.
//!
//! A code object is a VECSXP that counts no references (`NO_TRACKREFS`: the
//! constants it holds are the source's own objects, and GNU R's `REFCNT` of
//! them must not change because robigo compiled them, §4):
//!
//! | element | contents |
//! |---|---|
//! | 0 | a RAWSXP: the [`Header`], then the instructions at [`INS_OFFSET`] |
//! | 1 | the source: the expression compiled |
//! | 2.. | the constant pool, indexed by the instructions' `e`/`f` |
//!
//! Code objects are private: they hang off the 4th node word of the
//! expression they compile (a LANGSXP, as GNU R's byte code hangs off a
//! closure's body) or of a closure, which no R code and no C API reads, so
//! nothing but [`build`] ever writes an instruction blob. The VM derives
//! raw pointers into a code object only between safepoints and reloads them
//! from the rooted code register after each (docs/g3.md §3: no derived
//! pointer across a safepoint).

#![allow(unsafe_code)]

use crate::ir::{Header, INS_OFFSET, Ins, MAGIC, Profile};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp, flags};
use robigo_rtcore::Cx;

/// The pool index of the first constant.
pub const FIRST_CONST: usize = 4;
const FORMALS: usize = 2;
const SHAPE: usize = 3;

/// A compiled code object before it is allocated.
pub struct Built {
    pub ins: Vec<Ins>,
    /// The pool from index [`FIRST_CONST`].
    pub consts: Vec<Sexp>,
    pub nregs: u16,
    pub max_depth: u16,
    pub profile: Profile,
    /// The symbols the code assigns with `<-`/`=` or binds by `for`, each
    /// once (a closure's locals).
    pub locals: Vec<Sexp>,
}

/// Allocate the code object of `b` for `source`, under `&Cx`; with
/// `formals`, a closure's, with the frame shape of its formals and locals.
///
/// # Safety
/// `source`, `formals` and every constant are live objects of `cx`'s
/// isolate, reachable from roots (or immortal) for this call.
pub unsafe fn build(cx: &Cx, b: &Built, source: Sexp, formals: Option<Sexp>) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    let shape = match formals {
        // SAFETY: the caller's contract: a live pairlist of formals.
        Some(f) => unsafe { make_shape(cx, f, &b.locals)? },
        None => nil,
    };
    let bytes = INS_OFFSET + b.ins.len() * size_of::<Ins>();
    let blob = cx.alloc_vector_raw(RAWSXP, bytes)?;
    let code = cx.alloc_vector_raw(VECSXP, FIRST_CONST + b.consts.len())?;
    let hdr = Header {
        magic: MAGIC,
        profile: b.profile,
        _pad: [0; 3],
        nregs: b.nregs as u32,
        nins: b.ins.len() as u32,
        max_depth: b.max_depth as u32,
    };
    // SAFETY: both are fresh: the blob has `bytes` bytes (16-aligned data),
    // the pool its elements; the elements are live objects (the caller's
    // contract), stored into a fresh VECSXP that counts no references, so
    // the stores neither count nor take the barrier.
    unsafe {
        code.set_flag(flags::NO_TRACKREFS, true);
        let p = blob.std_data();
        (p as *mut Header).write(hdr);
        let ins = p.add(INS_OFFSET) as *mut Ins;
        std::ptr::copy_nonoverlapping(b.ins.as_ptr(), ins, b.ins.len());
        code.set_elt(0, blob);
        code.set_elt(1, source);
        code.set_elt(FORMALS, formals.unwrap_or(nil));
        code.set_elt(SHAPE, shape);
        for (i, &k) in b.consts.iter().enumerate() {
            code.set_elt(FIRST_CONST + i, k);
        }
    }
    Ok(code)
}

/// The raw view of a live code object, valid until the next safepoint.
#[derive(Clone, Copy)]
pub struct View {
    pub ins: *const Ins,
    pub consts: *const Sexp,
    pub nregs: usize,
    pub nins: usize,
    pub profile: Profile,
}

impl View {
    /// The view of `code`.
    ///
    /// # Safety
    /// `code` is a live code object ([`build`]).
    #[inline(always)]
    pub unsafe fn of(code: Sexp) -> View {
        // SAFETY: the caller's contract: element 0 is the blob, written by
        // `build` from valid instructions, and nothing writes it after.
        unsafe {
            let consts = code.std_data() as *const Sexp;
            let blob = *consts;
            let p = blob.std_data();
            let hdr = &*(p as *const Header);
            debug_assert_eq!(hdr.magic, MAGIC);
            View {
                ins: p.add(INS_OFFSET) as *const Ins,
                consts,
                nregs: hdr.nregs as usize,
                nins: hdr.nins as usize,
                profile: hdr.profile,
            }
        }
    }

    /// Instruction `pc`.
    ///
    /// # Safety
    /// `pc` is below `nins`, and the code object is still live and unmoved.
    #[inline(always)]
    pub unsafe fn at(&self, pc: usize) -> Ins {
        debug_assert!(pc < self.nins);
        // SAFETY: the caller's contract.
        unsafe { *self.ins.add(pc) }
    }

    /// Constant `k` of the pool.
    ///
    /// # Safety
    /// As for [`View::at`]; `k` is a pool index the compiler wrote.
    #[inline(always)]
    pub unsafe fn k(&self, k: u16) -> Sexp {
        // SAFETY: the caller's contract.
        unsafe { *self.consts.add(k as usize) }
    }
}

/// Whether `x` (the 4th word of an expression or closure) is a code object
/// of `profile`.
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
pub unsafe fn is_code(x: Sexp, profile: Profile) -> bool {
    // SAFETY: the caller's contract; only `build` makes NO_TRACKREFS
    // VECSXPs that hang off a node's 4th word.
    unsafe {
        x.typ() == VECSXP && x.has_flag(flags::NO_TRACKREFS) && x.std_len() >= FIRST_CONST as isize && {
            let blob = *(x.std_data() as *const Sexp);
            blob.typ() == RAWSXP && {
                let hdr = &*(blob.std_data() as *const Header);
                hdr.magic == MAGIC && hdr.profile == profile
            }
        }
    }
}

/// A closure's frame shape: its formals' symbols, then `locals` not among
/// them (N1b's `new_shape`).
///
/// # Safety
/// `formals` is a live pairlist whose tags are symbols; `locals` live
/// symbols.
unsafe fn make_shape(cx: &Cx, formals: Sexp, locals: &[Sexp]) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    let mut syms: Vec<robigo_rtcore::Sym<'_>> = Vec::new();
    let mut f = formals;
    // SAFETY: the caller's contract; symbols are immortal.
    unsafe {
        while f != nil {
            let s = robigo_rtcore::Ref::from_sexp(f.tag(), cx).as_sym().expect("robigo: a formal that is not a symbol");
            if !syms.contains(&s) {
                syms.push(s);
            }
            f = f.cdr();
        }
        for &l in locals {
            let s = robigo_rtcore::Ref::from_sexp(l, cx).as_sym().expect("robigo: a local that is not a symbol");
            if !syms.contains(&s) {
                syms.push(s);
            }
        }
    }
    Ok(cx.new_shape(&syms)?.sexp())
}

/// Whether `x` is a closure's code object of `profile` compiled for the
/// formals `formals` (the same pairlist object).
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
pub unsafe fn is_closure_code(x: Sexp, profile: Profile, formals: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        is_code(x, profile)
            && *(x.std_data() as *const Sexp).add(FORMALS) == formals
            && *(x.std_data() as *const Sexp).add(SHAPE) != Sexp::NULL
            && (*(x.std_data() as *const Sexp).add(SHAPE)).typ() == VECSXP
    }
}

/// A closure's code object's frame shape.
///
/// # Safety
/// `code` is a live closure code object ([`is_closure_code`]).
#[inline(always)]
pub unsafe fn shape(code: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe { *(code.std_data() as *const Sexp).add(SHAPE) }
}
