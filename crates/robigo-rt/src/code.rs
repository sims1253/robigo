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
//! | 2 | a closure's formals (R_NilValue otherwise) |
//! | 3 | a closure's frame shape (R_NilValue otherwise) |
//! | 4 | the call sites' match plans: a RAWSXP of plain data (`args::Plan`), or R_NilValue |
//! | 5.. | the constant pool, indexed by the instructions' `e`/`f` |
//!
//! A code object's header carries a stamp, a per-isolate counter that
//! never repeats: caches that depend on a code object (a call site's match
//! plan depends on the callee's formals, which its code was compiled for)
//! key on the stamp, never on an address (docs/g3.md §9.3).
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
pub const FIRST_CONST: usize = 5;
const FORMALS: usize = 2;
const SHAPE: usize = 3;
const PLANS: usize = 4;

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
    /// Closure call sites ([`crate::ir::Op::Call`]'s `c`).
    pub nsites: u16,
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
    let plans = if b.nsites > 0 {
        let p = cx.alloc_vector_raw(RAWSXP, b.nsites as usize * crate::args::PLAN_BYTES)?;
        // SAFETY: fresh plain data: every plan starts empty (stamp 0)
        unsafe { std::ptr::write_bytes(p.std_data(), 0, b.nsites as usize * crate::args::PLAN_BYTES) };
        p
    } else {
        nil
    };
    let code = cx.alloc_vector_raw(VECSXP, FIRST_CONST + b.consts.len())?;
    let hdr = Header {
        magic: MAGIC,
        profile: b.profile,
        _pad: [0; 3],
        nregs: b.nregs as u32,
        nins: b.ins.len() as u32,
        max_depth: b.max_depth as u32,
        nsites: b.nsites as u32,
        stamp: crate::prims::state(cx).next_stamp(),
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
        code.set_elt(PLANS, plans);
        for (i, &k) in b.consts.iter().enumerate() {
            code.set_elt(FIRST_CONST + i, k);
        }
    }
    Ok(code)
}

/// A copy of closure code object `code` for the formals `formals` (the
/// JIT cache's reuse of a body's code by a closure whose formals are other
/// objects, `R_cmpfun`): the same instructions and constants, a new stamp
/// (match plans key on it), empty match plans, and the frame shape of the
/// new formals followed by the code's locals.
///
/// # Safety
/// `code` is a live closure code object and `formals` a live pairlist of
/// symbol-tagged cells, both of `cx`'s isolate and reachable from roots.
pub unsafe fn reshape(cx: &Cx, code: Sexp, formals: Sexp) -> Result<Sexp, NeedGc> {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let el = |x: Sexp, i: usize| *(x.std_data() as *const Sexp).add(i);
        let old_formals = el(code, FORMALS);
        let old_shape = el(code, SHAPE);
        let mut ftags = Vec::new();
        let mut f = old_formals;
        while f != nil {
            ftags.push(f.tag());
            f = f.cdr();
        }
        let locals: Vec<Sexp> =
            (0..old_shape.std_len() as usize).map(|i| el(old_shape, i)).filter(|s| !ftags.contains(s)).collect();
        let shape = make_shape(cx, formals, &locals)?;
        let blob = el(code, 0);
        let bytes = blob.std_len() as usize;
        let nb = cx.alloc_vector_raw(RAWSXP, bytes)?;
        std::ptr::copy_nonoverlapping(el(code, 0).std_data(), nb.std_data(), bytes);
        (*(nb.std_data() as *mut Header)).stamp = crate::prims::state(cx).next_stamp();
        let plans = el(code, PLANS);
        let np = if plans.typ() == RAWSXP {
            let n = plans.std_len() as usize;
            let p = cx.alloc_vector_raw(RAWSXP, n)?;
            std::ptr::write_bytes(p.std_data(), 0, n);
            p
        } else {
            nil
        };
        let len = code.std_len() as usize;
        let c = cx.alloc_vector_raw(VECSXP, len)?;
        c.set_flag(flags::NO_TRACKREFS, true);
        for i in 0..len {
            let v = match i {
                0 => nb,
                FORMALS => formals,
                SHAPE => shape,
                PLANS => np,
                _ => el(code, i),
            };
            c.set_elt(i, v);
        }
        Ok(c)
    }
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

/// A code object's stamp ([`Header::stamp`]).
///
/// # Safety
/// `code` is a live code object.
#[inline(always)]
pub unsafe fn stamp(code: Sexp) -> u64 {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe { (*((*(code.std_data() as *const Sexp)).std_data() as *const Header)).stamp }
}

/// Call site `site`'s match-plan bytes in `code`, or None if it has none.
///
/// # Safety
/// `code` is a live code object; the pointer is valid until the next
/// safepoint (the RAWSXP may move then).
#[inline(always)]
pub unsafe fn plan_bytes(code: Sexp, site: u16) -> Option<*mut u8> {
    // SAFETY: the caller's contract.
    unsafe {
        let plans = *(code.std_data() as *const Sexp).add(PLANS);
        if plans.typ() != RAWSXP {
            return None;
        }
        let at = site as usize * crate::args::PLAN_BYTES;
        if at + crate::args::PLAN_BYTES > plans.std_len() as usize {
            return None;
        }
        Some(plans.std_data().add(at))
    }
}
