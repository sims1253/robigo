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
//! | 5 | the same expression's code of the other profile, or R_NilValue ([`cached`]) |
//! | 6 | the shape the `GetVar`/`SetVar` slot indices are for: a closure's own; for other code, the shape of the frame it runs in, or R_NilValue ([`set_index_shape`]) |
//! | 7 | `Eval` code: a RAWSXP of one `i32` per instruction, its protect offset ([`View::ppk`]); R_NilValue for `Bc` code |
//! | 8.. | the constant pool, indexed by the instructions' `e`/`f` |
//!
//! A code object's header carries a stamp, a per-isolate counter that
//! never repeats: caches that depend on a code object (a call site's match
//! plan depends on the callee's formals, which its code was compiled for)
//! key on the stamp, never on an address (docs/g3.md §9.3).
//!
//! The stamp also dates the code against writes into what it was made
//! from (§9.3's tracked writes): [`build`] marks the cells of an
//! expression compiled for GNU R's AST, and of a closure's formals
//! (`Cx::mark_code_source`); a write into a marked cell through a setter
//! raises the isolate's floor for that kind of code, and [`is_code`] and
//! [`is_closure_code`] refuse code stamped below it, which is then compiled
//! again from the cells as they are. Byte code is never refused for a
//! write into its expression: GNU R's byte code is an object of its own,
//! which such a write does not change.
//!
//! Code objects are private: they hang off the 4th node word of the
//! expression they compile (a LANGSXP, as GNU R's byte code hangs off a
//! closure's body) or of a closure, which no R code and no C API reads, so
//! nothing but [`build`] ever writes an instruction blob ([`set_index_shape`]
//! writes the slot indices of one before it first runs). The VM derives
//! raw pointers into a code object only between safepoints and reloads them
//! from the rooted code register after each (docs/g3.md §3: no derived
//! pointer across a safepoint).

#![allow(unsafe_code)]

use crate::ir::{Header, INS_OFFSET, Ins, MAGIC, Profile};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp, flags};
use robigo_rtcore::{CodeSource, Cx};

/// The pool index of the first constant.
pub const FIRST_CONST: usize = 8;
const FORMALS: usize = 2;
const SHAPE: usize = 3;
const PLANS: usize = 4;
const OTHER: usize = 5;
const IXSHAPE: usize = 6;
const PPK: usize = 7;

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
    /// `Eval` code: each instruction's protect offset ([`View::ppk`]), one
    /// per instruction; empty for `Bc` code (byte code keeps the excess).
    pub ppk: Vec<i32>,
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
    debug_assert!(b.ppk.is_empty() || b.ppk.len() == b.ins.len());
    let bytes = INS_OFFSET + b.ins.len() * size_of::<Ins>();
    let blob = cx.alloc_vector_raw(RAWSXP, bytes)?;
    let ppk = if b.ppk.is_empty() {
        nil
    } else {
        let t = cx.alloc_vector_raw(RAWSXP, b.ppk.len() * size_of::<i32>())?;
        // SAFETY: fresh plain data of that many bytes (16-aligned).
        unsafe { std::ptr::copy_nonoverlapping(b.ppk.as_ptr(), t.std_data() as *mut i32, b.ppk.len()) };
        t
    };
    let plans = if b.nsites > 0 {
        let p = cx.alloc_vector_raw(RAWSXP, b.nsites as usize * crate::args::PLAN_BYTES)?;
        // SAFETY: fresh plain data: every plan starts empty (stamp 0)
        unsafe { std::ptr::write_bytes(p.std_data(), 0, b.nsites as usize * crate::args::PLAN_BYTES) };
        p
    } else {
        nil
    };
    let code = cx.alloc_vector_raw(VECSXP, FIRST_CONST + b.consts.len())?;
    // the cells this code stands for (docs/g3.md §9.3): GNU R walks an AST
    // live, and reads a closure's formals at every call
    // SAFETY: the caller's contract: live objects of this isolate.
    unsafe {
        if b.profile == Profile::Eval {
            cx.mark_code_source(robigo_rtcore::Ref::from_sexp(source, cx), CodeSource::Ast);
        }
        if let Some(f) = formals {
            cx.mark_code_source(robigo_rtcore::Ref::from_sexp(f, cx), CodeSource::Formals);
        }
    }
    let hdr = Header {
        magic: MAGIC,
        profile: b.profile,
        inline: 0,
        _pad: [0; 2],
        nregs: b.nregs as u32,
        nins: b.ins.len() as u32,
        max_depth: b.max_depth as u32,
        nsites: b.nsites as u32,
        stamp: cx.next_code_stamp(),
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
        index_slots(cx, ins, b.ins.len(), shape, |e| b.consts[e - FIRST_CONST]);
        code.set_elt(0, blob);
        code.set_elt(1, source);
        code.set_elt(FORMALS, formals.unwrap_or(nil));
        code.set_elt(SHAPE, shape);
        code.set_elt(PLANS, plans);
        code.set_elt(OTHER, nil);
        code.set_elt(IXSHAPE, shape);
        code.set_elt(PPK, ppk);
        for (i, &k) in b.consts.iter().enumerate() {
            code.set_elt(FIRST_CONST + i, k);
        }
    }
    Ok(code)
}

/// Write each `GetVar` and `SetVar` instruction's slot index (plus one)
/// into its `f`: the position of its symbol (`sym_of(e)`) in `shape`, or 0
/// when the symbol has no slot there (or `shape` is R_NilValue). The VM
/// reads and stores the slot directly when the frame is of this shape. A
/// `GetFun`'s `f` is 1 when `shape` holds symbols and not its name.
///
/// # Safety
/// `ins` points to `n` instructions; `shape` is R_NilValue or a live shape
/// (a VECSXP of symbols).
unsafe fn index_slots(cx: &Cx, ins: *mut Ins, n: usize, shape: Sexp, sym_of: impl Fn(usize) -> Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        let syms: &[Sexp] = if shape == cx.nil().sexp() {
            &[]
        } else {
            std::slice::from_raw_parts(shape.std_data() as *const Sexp, shape.std_len() as usize)
        };
        for k in 0..n {
            let i = &mut *ins.add(k);
            if matches!(i.op, crate::ir::Op::GetVar | crate::ir::Op::SetVar) {
                let sym = sym_of(i.e as usize);
                i.f = match syms.iter().position(|&s| s == sym) {
                    Some(at) if at < u16::MAX as usize => at as u16 + 1,
                    _ => 0,
                };
            } else if i.op == crate::ir::Op::GetFun {
                // 1: the shape lacks the name (the lookup may start in the
                // frame's enclosure, `Cx::fun_lookup_skip_raw`)
                let sym = sym_of(i.e as usize);
                i.f = u16::from(!syms.is_empty() && !syms.contains(&sym));
            }
        }
    }
}

/// Make the slot indices of `code`, code that is not a closure's (a
/// promise's or an argument's), those of `shape`: the shape of the frames
/// it runs in (the closure frame its expression belongs to). Its
/// `GetVar`/`SetVar` then read and store those frames' slots directly, and
/// its `GetFun`s pass over such a frame when it lacks the name, as a
/// closure's code does in its own frame; in a frame of another shape the
/// VM's one compare fails and the full lookup runs. Before `code` first
/// runs.
///
/// # Safety
/// `code` is a live code object of `cx`'s isolate that has not run, and
/// `shape` a live shape (a VECSXP of symbols) or R_NilValue.
pub unsafe fn set_index_shape(cx: &Cx, code: Sexp, shape: Sexp) {
    // SAFETY: the caller's contract; the blob's instructions are written in
    // place (plain data), the element stored into a VECSXP that counts no
    // references.
    unsafe {
        let blob = *(code.std_data() as *const Sexp);
        let nins = (*(blob.std_data() as *const Header)).nins as usize;
        index_slots(cx, blob.std_data().add(INS_OFFSET) as *mut Ins, nins, shape, |e| {
            *(code.std_data() as *const Sexp).add(e)
        });
        code.set_elt(IXSHAPE, shape);
    }
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
        cx.mark_code_source(robigo_rtcore::Ref::from_sexp(formals, cx), CodeSource::Formals);
        let blob = el(code, 0);
        let bytes = blob.std_len() as usize;
        let nb = cx.alloc_vector_raw(RAWSXP, bytes)?;
        std::ptr::copy_nonoverlapping(el(code, 0).std_data(), nb.std_data(), bytes);
        (*(nb.std_data() as *mut Header)).stamp = cx.next_code_stamp();
        // the inline verdict was the old formals': the copy is judged again
        // for its own (crate::inline; a fresh header's 0)
        (*(nb.std_data() as *mut Header)).inline = 0;
        // the slot indices are the old shape's: index them for the new one
        let nins = (*(nb.std_data() as *const Header)).nins as usize;
        index_slots(cx, nb.std_data().add(INS_OFFSET) as *mut Ins, nins, shape, |e| el(code, e));
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
                IXSHAPE => shape,
                PLANS => np,
                OTHER => nil,
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
    /// The shape the slot indices are for: a closure's frame shape, the
    /// shape of the frames other code runs in ([`set_index_shape`]), or
    /// R_NilValue.
    pub shape: Sexp,
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
                shape: *consts.add(IXSHAPE),
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

    /// `Eval` code's protect offset of instruction `pc`: GNU R's
    /// evaluator protections minus `R_EvalDepth` where it calls out, less
    /// the activation's at its root ([`crate::vm::Act::e0`];
    /// `robigo_rtcore`'s ppacct).
    ///
    /// # Safety
    /// As for [`View::at`], and the code is `Eval` code.
    #[inline(always)]
    pub unsafe fn ppk(&self, pc: usize) -> i32 {
        debug_assert!(pc < self.nins && self.profile == Profile::Eval);
        // SAFETY: the caller's contract: `build` made the element, one per
        // instruction (a code object's elements are fixed).
        unsafe { *((*self.consts.add(PPK)).std_data() as *const i32).add(pc) }
    }
}

/// Whether `x` (the 4th word of an expression or closure) is a code object
/// of `profile` that still stands for what GNU R runs: code for GNU R's
/// AST is stale once a cell of its source was written
/// (`Cx::code_is_current`, docs/g3.md §9.3).
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
pub unsafe fn is_code(cx: &Cx, x: Sexp, profile: Profile) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_code_object(x, profile) && cx.code_is_current(stamp(x), profile == Profile::Eval, false) }
}

/// Whether `x` is a code object of `profile`, current or not.
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
unsafe fn is_code_object(x: Sexp, profile: Profile) -> bool {
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

/// The code of `profile` that `x` (a call's 4th word: R_NilValue or a code
/// object) holds: itself, or the code of the other profile it carries. A
/// call evaluated by both the AST and byte code (an argument's expression
/// once its function is compiled) keeps both, so neither replaces the
/// other's decisions.
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
pub unsafe fn cached(cx: &Cx, x: Sexp, profile: Profile) -> Option<Sexp> {
    // SAFETY: the caller's contract; a code object's elements are live.
    unsafe {
        if is_code_object(x, profile) {
            return cx.code_is_current(stamp(x), profile == Profile::Eval, false).then_some(x);
        }
        if is_code_object(x, other(profile)) {
            let o = *(x.std_data() as *const Sexp).add(OTHER);
            if is_code(cx, o, profile) {
                return Some(o);
            }
        }
        None
    }
}

/// Keep `code` (of its profile) in the 4th word of the call `e`, beside the
/// other profile's code it holds ([`cached`]).
///
/// # Safety
/// `e` is a live call and `code` a live code object of the same heap.
pub unsafe fn cache_on(e: Sexp, code: Sexp) {
    // SAFETY: the caller's contract; traced words, stores with the barrier.
    unsafe {
        let p = View::of(code).profile;
        let x = e.aux();
        if is_code_object(x, other(p)) {
            x.set_elt(OTHER, code);
        } else {
            if is_code_object(x, p) {
                // a stale code object of this profile: the other profile's
                // code it holds stays
                code.set_elt(OTHER, *(x.std_data() as *const Sexp).add(OTHER));
            }
            e.set_aux(code);
        }
    }
}

fn other(p: Profile) -> Profile {
    match p {
        Profile::Eval => Profile::Bc,
        Profile::Bc => Profile::Eval,
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
/// formals `formals` (the same pairlist object), current: no cell of any
/// closure's formals was written since ([`is_code`]'s rule for the AST
/// too).
///
/// # Safety
/// `x` is live (or R_NilValue).
#[inline(always)]
pub unsafe fn is_closure_code(cx: &Cx, x: Sexp, profile: Profile, formals: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        is_code_object(x, profile)
            && cx.code_is_current(stamp(x), profile == Profile::Eval, true)
            && *(x.std_data() as *const Sexp).add(FORMALS) == formals
            && *(x.std_data() as *const Sexp).add(SHAPE) != Sexp::NULL
            && (*(x.std_data() as *const Sexp).add(SHAPE)).typ() == VECSXP
    }
}

/// Whether `x` is a closure's byte code for `formals` that a write into a
/// closure's formals made stale, and nothing else: GNU R keeps such byte
/// code (its body), and reads the formals at the call, so [`reshape`]
/// makes the code for them again with the same instructions.
///
/// # Safety
/// `x` is live (or R_NilValue).
pub unsafe fn formals_stale_bc(cx: &Cx, x: Sexp, formals: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        is_code_object(x, Profile::Bc)
            && !cx.code_is_current(stamp(x), false, true)
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
