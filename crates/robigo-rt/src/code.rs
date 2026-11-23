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
pub(crate) const IXSHAPE: usize = 6;
const PPK: usize = 7;

/// A compiled code object before it is allocated.
#[derive(Clone)]
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

/// Code that passed [`Verified::new`]'s checks: the only input [`build`]
/// takes, so every code object the VM runs is well formed (#738).
///
/// Its invariant, which the VM's raw reads rely on: every register operand
/// (and every register an instruction reads past one, its argument window,
/// its names, `for`'s state) is below `nregs`, and `nregs` covers the
/// fixed registers below [`R_FIRST`]; every constant operand indexes the
/// pool; every jump target is an instruction, and the code cannot run off
/// its end; a `Switch`'s jump tables are there, one `Jmp` each; a call
/// site is below `nsites` (or [`crate::ir::NO_SITE`]); an `LdScalar`
/// constant is a plain scalar.
///
/// **What it does not check**, and the VM also relies on: the type of any
/// other constant (a symbol where `GetVar` reads one, a call where `Call`
/// walks one, ...), and the state the stateful operations keep in
/// registers: that `for`'s state window is written only by `ForPrep` and
/// `ForStep` and that every `ForStep` and `ForDone` comes after its
/// `ForPrep`; the same for a subset's (`StartSubset`, `DfltSubset`) and a
/// loop region's. Those hold because robigo-rt's own translator and
/// compiler made the code, by construction: so [`Verified::new`] is for
/// their code only, and code from anywhere else comes in through
/// [`Verified::from_cached_translation`], whose caller promises the rest.
pub struct Verified(Built);

/// Why [`Verified::new`] refused code: the instruction and the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    pub at: usize,
    pub why: &'static str,
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "robigo: malformed code at instruction {}: {}", self.at, self.why)
    }
}

impl Verified {
    /// Check `b`, code this process's translator or compiler just made, in
    /// one pass over its instructions (#738).
    pub fn new(b: Built) -> Result<Verified, Malformed> {
        check(&b)?;
        Ok(Verified(b))
    }

    /// Check `b`, code read back from the translation cache
    /// (`crate::tcache`), as [`Verified::new`] checks code.
    ///
    /// # Safety
    /// `b` is code this build's translator made and wrote: an intact entry
    /// of this binary's own cache for this tree, so that what
    /// [`Verified::new`] does not check (the stateful operations' register
    /// invariants, [`Verified`]) holds by construction. Its constants have
    /// passed `tcache`'s typed check. Bytes from anywhere else (a file
    /// another build wrote, a hand-edited entry with a checksum made to
    /// match) carry no such promise: a forged entry can break the VM's
    /// register invariants (a `ForStep` whose state another instruction
    /// overwrote reads out of bounds), and this constructor does not detect it.
    pub(crate) unsafe fn from_cached_translation(b: Built) -> Result<Verified, Malformed> {
        check(&b)?;
        Ok(Verified(b))
    }

    /// The checked code.
    pub fn built(&self) -> &Built {
        &self.0
    }
}

/// [`Verified::new`]'s checks.
fn check(b: &Built) -> Result<(), Malformed> {
    use crate::ir::{NO_CONST, NO_REG, Op, R_FIRST, SPECIAL_INTERNAL};
    let n = b.ins.len();
    let nregs = b.nregs as usize;
    let npool = FIRST_CONST + b.consts.len();
    let bad = |at: usize, why: &'static str| Err(Malformed { at, why });
    if nregs < R_FIRST as usize {
        return bad(0, "fewer registers than the fixed ones");
    }
    if !b.ppk.is_empty() && b.ppk.len() != n {
        return bad(0, "protect offsets that do not match the instructions");
    }
    match b.ins.last().map(|i| i.op) {
        Some(Op::Ret | Op::Return | Op::LoopEnd | Op::Jmp | Op::BackJmp | Op::BreakJump | Op::ErrorCall) => {}
        _ => return bad(n.saturating_sub(1), "the code can run off its end"),
    }
    let mut at = 0;
    while at < n {
        let i = &b.ins[at];
        let reg = |r: u16| (r as usize) < nregs;
        let regs = |r: u16, k: usize| r as usize + k <= nregs;
        let k = |x: u16| (FIRST_CONST..npool).contains(&(x as usize));
        let tgt = |t: usize| t < n;
        let window = |r: u16, c: u16| regs(r, c as usize * if i.fl & crate::ir::F_NAMES != 0 { 2 } else { 1 });
        // (F_CLR_C clears register c, wherever the op reads it)
        if i.fl & crate::ir::F_CLR_C != 0 && !reg(i.c) {
            return bad(at, "a cleared register out of range");
        }
        let ok = match i.op {
            Op::Nop | Op::LoopEnd | Op::PpRestore | Op::SetVis | Op::BreakJump => true,
            Op::LdConst | Op::GetVar | Op::GetDdVar | Op::GetVarMissOk | Op::Quote | Op::EvalConst => {
                reg(i.a) && k(i.e)
            }
            Op::LdScalar => {
                // SAFETY: the pool's constants are live (the builder's
                // contract, as [`build`]'s).
                reg(i.a) && k(i.e) && unsafe { plain_scalar(b.consts[i.e as usize - FIRST_CONST]) }
            }
            Op::SetVar | Op::SetVar2 => reg(i.a) && k(i.e),
            Op::Jmp | Op::BackJmp => tgt(i.target()),
            Op::BrFalse => reg(i.b) && k(i.e) && tgt(i.target()),
            Op::Guard => (i.a == 0 || reg(i.a)) && k(i.e) && k(i.c) && tgt(i.target()),
            Op::GetFun => reg(i.a) && k(i.e) && k(i.c),
            Op::Call => reg(i.a) && reg(i.b) && k(i.e) && (i.c < b.nsites || i.c == crate::ir::NO_SITE),
            Op::CallCodes => reg(i.a) && reg(i.b) && k(i.e) && k(i.f) && (i.c < b.nsites || i.c == crate::ir::NO_SITE),
            Op::CallBuiltin | Op::Internal => {
                reg(i.a)
                    && k(i.e)
                    && k(i.f)
                    && (i.g == 0 || k(i.g))
                    && (i.c == SPECIAL_INTERNAL || i.c == u16::MAX || window(i.b, i.c))
            }
            Op::Arith | Op::Relop | Op::Logic => reg(i.a) && reg(i.b) && reg(i.c) && regs(i.b, 2) && k(i.e) && k(i.f),
            Op::Arith1 | Op::Not => reg(i.a) && reg(i.b) && k(i.e) && k(i.f),
            Op::AndOr1 => reg(i.a) && reg(i.b) && k(i.e) && tgt(i.target()),
            Op::AndOr2 => reg(i.a) && reg(i.b) && k(i.e),
            Op::MkClosure => reg(i.a) && k(i.e) && k(i.f) && (i.c == 0 || k(i.c)),
            Op::Missing => reg(i.a) && k(i.e) && k(i.f),
            Op::Return | Op::Ret => reg(i.b),
            Op::PpCheck => reg(i.b) && k(i.e),
            // (the VM goes on after the region's end, its target)
            Op::Loop => i.a as usize <= nregs && tgt(i.target() + 1),
            Op::ForPrep => regs(i.a, crate::compile::FOR_STATE) && k(i.e) && k(i.c),
            Op::ForStep => regs(i.a, crate::compile::FOR_STATE) && k(i.e) && k(i.c) && tgt(i.target()),
            Op::ForDone | Op::Clear | Op::PpSave => reg(i.a),
            Op::ClearRange => i.a <= i.b && i.b as usize <= nregs,
            Op::Move => reg(i.a) && reg(i.b),
            Op::ErrorCall => k(i.e) && k(i.f) && (i.c == 0 || k(i.c)),
            Op::SpecialFn => reg(i.a) && k(i.e) && k(i.c),
            Op::ComplexAssign | Op::BcAssign => reg(i.a) && reg(i.b) && k(i.e),
            Op::BaseGuard | Op::TopLoop => reg(i.a) && k(i.e) && tgt(i.target()),
            Op::StartSubset => reg(i.a) && reg(i.b) && k(i.e) && tgt(i.target()),
            Op::DfltSubset => reg(i.a) && k(i.e) && window(i.b, i.c),
            Op::Dollar => reg(i.a) && reg(i.b) && k(i.e) && k(i.c),
            Op::Math1 => {
                let kinds =
                    i.g < crate::ir::MATH_SQRT || (crate::ir::MATH_SQRT..=crate::ir::MATH_LOGBASE).contains(&i.g);
                let n = if i.g == crate::ir::MATH_LOGBASE { 2 } else { 1 };
                reg(i.a)
                    && k(i.e)
                    && (i.f == 0 || k(i.f))
                    && (i.f != 0 || i.g >= crate::ir::MATH_LOG)
                    && i.c == n
                    && regs(i.b, n as usize)
                    && kinds
                    && (i.g >= crate::ir::MATH_SQRT || (i.g as usize) < robigo_rtbuiltins::math::BC_MATH1.len())
            }
            Op::Resume => reg(i.a) && (i.b == NO_REG || reg(i.b)) && k(i.e) && tgt(i.target()),
            Op::LoopResume => reg(i.a) && k(i.e) && (i.c == NO_CONST || (k(i.c) && k(i.b))) && tgt(i.target() + 1),
            Op::Switch => {
                if !(reg(i.b) && k(i.e) && k(i.c)) {
                    false
                } else {
                    // the jump tables that follow: one per name, then `f`
                    // numeric entries, each a Jmp
                    let names = b.consts[i.c as usize - FIRST_CONST];
                    // SAFETY: a live constant of the pool (as above).
                    let nc = unsafe { if names.typ() == NILSXP { 0 } else { names.std_len() as usize } };
                    let end = at + 1 + nc + i.f as usize;
                    if end > n || !b.ins[at + 1..end].iter().all(|j| j.op == Op::Jmp) {
                        return bad(at, "a switch without its jump tables");
                    }
                    true
                }
            }
        };
        if !ok {
            return bad(at, "an operand out of range");
        }
        // a builtin's call and an inlined builtin's slow path pass the
        // window's registers as the arguments, write the value after and
        // then clear the window (`call_builtin_regs`): the value's register
        // is outside it, and a binary operation's operands are its two
        // registers
        let w = i.fl & crate::ir::F_NAMES != 0;
        let outside = |lo: u16, k: usize| !(lo as usize..lo as usize + k).contains(&(i.a as usize));
        let apart = match i.op {
            Op::Arith | Op::Relop | Op::Logic => i.c as usize == i.b as usize + 1 && outside(i.b, 2),
            Op::Arith1 | Op::Not => outside(i.b, 1),
            Op::DfltSubset => outside(i.b, i.c as usize * if w { 2 } else { 1 }),
            Op::Math1 => outside(i.b, i.c as usize),
            Op::CallBuiltin | Op::Internal if i.c != SPECIAL_INTERNAL && i.c != u16::MAX => {
                outside(i.b, i.c as usize * if w { 2 } else { 1 })
            }
            _ => true,
        };
        if !apart {
            return bad(at, "a value register inside its operands' window");
        }
        at += 1;
    }
    Ok(())
}

/// A plain length-1 double, integer or logical with no attributes, its
/// payload inline: what `LdScalar` reads.
///
/// # Safety
/// `x` is live.
pub(crate) unsafe fn plain_scalar(x: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        matches!(x.typ(), REALSXP | INTSXP | LGLSXP)
            && !x.is_altrep()
            && !x.has_flag(flags::EXTDATA)
            && x.std_len() == 1
            && x.attrib_raw().typ() == NILSXP
    }
}

/// Allocate the code object of the verified code `v` for `source`, under
/// `&Cx`; with `formals`, a closure's, with the frame shape of its formals
/// and locals.
///
/// # Safety
/// `source`, `formals` and every constant are live objects of `cx`'s
/// isolate, reachable from roots (or immortal) for this call.
pub unsafe fn build(cx: &Cx, v: &Verified, source: Sexp, formals: Option<Sexp>) -> Result<Sexp, NeedGc> {
    let b = &v.0;
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
        inline_formals: 0,
        _pad: [0; 1],
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
        // the inline verdict (and its formal count) was the old formals':
        // the copy is judged again for its own (crate::inline; a fresh
        // header's 0)
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
            // (a code object and its blob keep their payloads inline:
            // `build` allocates them)
            let consts = code.inline_data() as *const Sexp;
            let blob = *consts;
            let p = blob.inline_data();
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

    /// Instruction `pc` if the code has one there.
    ///
    /// # Safety
    /// The code object is still live and unmoved.
    #[inline(always)]
    pub unsafe fn ins_at(&self, pc: usize) -> Option<&Ins> {
        // SAFETY: the caller's contract; `pc` is below `nins`.
        (pc < self.nins).then(|| unsafe { &*self.ins.add(pc) })
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
            // (`build`'s objects keep their payloads inline)
            let blob = *(x.inline_data() as *const Sexp);
            blob.typ() == RAWSXP && {
                let hdr = &*(blob.inline_data() as *const Header);
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
        is_code_object(x, profile) && cx.code_is_current(stamp(x), profile == Profile::Eval, true) && {
            let el = x.inline_data() as *const Sexp;
            let shape = *el.add(SHAPE);
            *el.add(FORMALS) == formals && shape != Sexp::NULL && shape.typ() == VECSXP
        }
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
        is_code_object(x, Profile::Bc) && !cx.code_is_current(stamp(x), false, true) && {
            // (`build`'s objects keep their payloads inline)
            let el = x.inline_data() as *const Sexp;
            let shape = *el.add(SHAPE);
            *el.add(FORMALS) == formals && shape != Sexp::NULL && shape.typ() == VECSXP
        }
    }
}

/// Whether the code object `x`, of a closure with formals `formals`, is
/// still current: [`is_closure_code`]'s checks that change after the code
/// was judged once (its stamp's currency, the formals it was made for), for
/// a site that remembers which code it judged ([`stamp`]; stamps are never
/// reused).
///
/// # Safety
/// `x` is a live code object of the byte-code profile.
#[inline(always)]
pub unsafe fn is_still_closure_code(cx: &Cx, x: Sexp, formals: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { cx.code_is_current(stamp(x), false, true) && *(x.inline_data() as *const Sexp).add(FORMALS) == formals }
}

/// A closure's code object's frame shape.
///
/// # Safety
/// `code` is a live closure code object ([`is_closure_code`]).
#[inline(always)]
pub unsafe fn shape(code: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe { *(code.inline_data() as *const Sexp).add(SHAPE) }
}

/// A code object's stamp ([`Header::stamp`]).
///
/// # Safety
/// `code` is a live code object.
#[inline(always)]
pub unsafe fn stamp(code: Sexp) -> u64 {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe { (*((*(code.inline_data() as *const Sexp)).inline_data() as *const Header)).stamp }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Op, R_FIRST};

    fn built(ins: Vec<Ins>, nregs: u16, nsites: u16) -> Built {
        Built {
            ins,
            consts: Vec::new(),
            nregs,
            max_depth: 0,
            profile: Profile::Bc,
            locals: Vec::new(),
            nsites,
            ppk: Vec::new(),
        }
    }

    fn ins(op: Op, a: u16, b: u16, c: u16) -> Ins {
        let mut i = Ins::new(op);
        i.a = a;
        i.b = b;
        i.c = c;
        i
    }

    fn why(b: Built) -> Option<&'static str> {
        Verified::new(b).err().map(|m| m.why)
    }

    #[test]
    fn well_formed_code_passes() {
        let r = R_FIRST;
        let code = vec![ins(Op::Move, r, r + 1, 0), ins(Op::Ret, 0, r, 0)];
        assert_eq!(why(built(code, r + 2, 0)), None);
    }

    #[test]
    fn the_verifier_refuses_what_the_vm_would_misread() {
        let r = R_FIRST;
        let ret = ins(Op::Ret, 0, r, 0);
        // the fixed registers, a register past the frame, a pool index past
        // the pool, a call site past the sites, a jump past the code
        assert!(why(built(vec![ret], r - 1, 0)).is_some());
        assert!(why(built(vec![ins(Op::Move, r + 5, r, 0), ret], r + 1, 0)).is_some());
        let mut ld = ins(Op::LdConst, r, 0, 0);
        ld.e = FIRST_CONST as u16;
        assert!(why(built(vec![ld, ret], r + 1, 0)).is_some());
        assert!(why(built(vec![ins(Op::Call, r, r, 1), ret], r + 1, 1)).is_some());
        let mut j = Ins::new(Op::Jmp);
        j.set_target(7);
        assert!(why(built(vec![j], r, 0)).is_some());
        // a builtin's argument window past the frame, names doubling it
        let mut cb = ins(Op::CallBuiltin, r, r, 2);
        cb.fl = crate::ir::F_NAMES;
        assert!(why(built(vec![cb, ret], r + 3, 0)).is_some());
        // code that runs off its end
        assert_eq!(why(built(vec![ins(Op::Move, r, r, 0)], r + 1, 0)), Some("the code can run off its end"));
        // a loop region whose end is the last instruction: nothing to go on
        // to
        let mut lp = Ins::new(Op::Loop);
        lp.set_target(1);
        assert!(why(built(vec![lp, Ins::new(Op::LoopEnd)], r, 0)).is_some());
    }
}
