//! Pure-closure inlining (docs/g3.md §9.2's carry-over from perf-vm; §12:
//! "a context per call GNU R makes one for, or deopt before it can be
//! read").
//!
//! A call from compiled code of a closure whose body is pure runs the body
//! inline when nothing a real call would make can be observed. A pure body
//! loads constants, reads its formals, its locals and its enclosure's
//! bindings, assigns its locals (not its formals), does scalar arithmetic
//! and comparisons, branches, and calls other pure closures. It has no
//! frame, context or promise objects. Its registers and its frame's slots
//! are local arrays, and its arguments are forced lazily, as the real call
//! would force them, from the caller's expressions.
//!
//! The run keeps GNU R's counts as the real call keeps them: the entry's
//! depth and `bcEval` poll count, each force's counts ([`early_force`]),
//! the evaluation points, `R_Visible`. It gives up before anything
//! observable: a count that would poll, the depth limit, a slow path (an
//! error, a warning, a coercion), a value it cannot read without running
//! code (an unforced promise, an active binding), or a call it cannot run
//! inline. Nothing has happened by then but those counts, so they are put
//! back and the real call runs from where the inline run began, after the
//! call's entry safepoint. The run allocates nothing and reaches no
//! safepoint, so nothing can collect while it holds raw objects.

#![allow(unsafe_code)]

use crate::code::View;
use crate::early::{early_force, read, symbol};
use crate::ir::*;
use crate::ops;
use crate::vm::Act;
use crate::vm::{num, num_slot};
use robigo_heap::types::*;
use robigo_heap::{Sexp, extra};
use robigo_rtbuiltins::kernel::relop::{EQOP, NEOP};
use robigo_rtcore::{Cx, Ref, Slot, Tag};

/// The registers an inlined body may use.
const MAX_REGS: usize = 16;
/// The formals an inlined closure may have.
const MAX_FORMALS: usize = 4;
/// The slots its frame shape may have: its formals, then its locals.
const MAX_SLOTS: usize = 4;
/// The registers argument code computed inline may use.
const MAX_ARG_REGS: usize = 8;
/// The most inline calls nested in one run (recursion beyond this gives up:
/// the outermost call then runs for real, and its body's calls start their
/// own runs).
const MAX_NEST: u32 = 12;

/// [`Header::inline`]: not decided yet, inlinable, not inlinable.
const UNDECIDED: u8 = 0;
const YES: u8 = 1;
const NO: u8 = 2;
// (a fresh header's byte is 0: `code::build` writes it so)
const _: () = assert!(UNDECIDED == 0);

/// Whether the closure code `code` may run inline, decided at the first
/// ask and kept in its header.
///
/// # Safety
/// `code` is a live closure code object of `op`, whose formals it was
/// compiled for.
#[inline(always)]
pub(crate) unsafe fn candidate(cx: &Cx, code: Sexp, op: Sexp) -> bool {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe {
        let hdr = (*(code.std_data() as *const Sexp)).std_data() as *mut Header;
        match (*hdr).inline {
            YES => true,
            NO => false,
            _ => decide(cx, code, op, hdr),
        }
    }
}

/// [`candidate`]'s first ask: the body qualifies when its frame shape is its
/// formals (no `...`) and then its locals, and every instruction is one an
/// inline run executes.
#[cold]
#[inline(never)]
unsafe fn decide(cx: &Cx, code: Sexp, op: Sexp, hdr: *mut Header) -> bool {
    // SAFETY: the caller's contract.
    let nf = unsafe { qualifies(cx, code, op) };
    // SAFETY: the header of a live blob, plain data.
    unsafe {
        (*hdr).inline_formals = nf.unwrap_or(0) as u8;
        (*hdr).inline = if nf.is_some() { YES } else { NO };
    }
    nf.is_some()
}

/// The formals of the closure code `code` (a [`candidate`]), which its
/// header keeps.
///
/// # Safety
/// `code` is a live code object judged inlinable.
#[inline(always)]
unsafe fn formals_of(code: Sexp) -> usize {
    // SAFETY: the caller's contract: element 0 is the blob with its header.
    unsafe { (*((*(code.std_data() as *const Sexp)).std_data() as *const Header)).inline_formals as usize }
}

/// Its formals when it qualifies.
///
/// # Safety
/// As for [`candidate`].
unsafe fn qualifies(cx: &Cx, code: Sexp, op: Sexp) -> Option<usize> {
    // SAFETY: the caller's contract.
    unsafe {
        let v = View::of(code);
        if v.profile != Profile::Bc || v.nregs > MAX_REGS {
            return None;
        }
        let shape = v.shape;
        if shape.typ() != VECSXP {
            return None;
        }
        let n = shape.std_len() as usize;
        if n > MAX_SLOTS {
            return None;
        }
        let shape_syms = shape.std_data() as *const Sexp;
        let dots = crate::prims::state(cx).syms.dots;
        // the shape's first slots are the formals, in order (`make_shape`),
        // and `...` is not one
        let mut f = op.car();
        let mut nf = 0;
        while f.typ() == LISTSXP {
            if nf == MAX_FORMALS || nf == n || f.tag() != *shape_syms.add(nf) || f.tag() == dots {
                return None;
            }
            nf += 1;
            f = f.cdr();
        }
        let in_shape = |s: Sexp| (0..n).any(|k| *shape_syms.add(k) == s);
        for pc in 0..v.nins {
            let i = v.at(pc);
            let ok = match i.op {
                Op::LdConst
                | Op::LdScalar
                | Op::Jmp
                | Op::BrFalse
                | Op::Call
                | Op::Arith
                | Op::Arith1
                | Op::Relop
                | Op::Not
                | Op::Logic
                | Op::AndOr1
                | Op::AndOr2
                | Op::Return
                | Op::Ret
                | Op::Clear
                | Op::ClearRange
                | Op::Move
                | Op::SetVis
                | Op::Nop => true,
                Op::GetVar => v.k(i.e).gp() & robigo_rtcore::DDVAL == 0 && (i.f as usize) <= n,
                // a store into a local's slot. Not a formal's: the real
                // call's promise for it, dropped from the frame by the
                // store, keeps the references to its expression and value
                // that GNU R's cleanup releases only for promises the frame
                // still binds (eval.c:R_CleanupEnvir), so the counts would
                // differ after an inline run
                Op::SetVar => v.k(i.e).gp() & robigo_rtcore::DDVAL == 0 && (i.f as usize) > nf && (i.f as usize) <= n,
                // a function the shape does not name, looked up from the
                // enclosure (with no formals the shape names none, and the
                // compiler marks none)
                Op::GetFun => i.f != 0 || n == 0,
                // a guard's symbol must not be a formal (the frame binds
                // nothing else)
                Op::Guard => !in_shape(v.k(i.e)),
                _ => false,
            };
            if !ok {
                return None;
            }
        }
        Some(nf)
    }
}

/// An argument of an inlined call.
#[derive(Clone, Copy)]
enum Arg {
    /// A value: a constant argument of compiled code (`PUSHCONSTARG`), or a
    /// forced one.
    Value(Slot),
    /// The caller's expression, not forced yet: a symbol or a call.
    Expr(Sexp),
    /// A missing formal's default, not forced yet: a constant, which the
    /// force evaluates in the frame (`eval`'s count, `R_Visible`).
    Default(Sexp),
    /// A local not assigned yet: the frame does not bind it, so a read goes
    /// on to the enclosure.
    Unbound,
}

/// Where an inlined call's arguments are evaluated: the real caller's
/// frame, or an inlined caller.
trait Scope {
    /// The value of `sym` read by code whose slot index for it is `slot`
    /// (0: none) for frames of `shape`, without running code: None when
    /// the read would have to force or check something.
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot>;

    /// The value of the argument expression `e` (a symbol or a call of
    /// compiled code), computed without running code, or None.
    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot>;
}

/// The real caller: its environment.
struct Real {
    rho: Sexp,
}

impl Scope for Real {
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot> {
        // SAFETY: the caller's rooted environment and a symbol; nothing
        // collects during the run. As `early::run`'s GetVar.
        unsafe {
            if sym.gp() & robigo_rtcore::DDVAL != 0 {
                return None;
            }
            let b = if slot != 0 { cx.frame_slot_raw(self.rho, shape, slot as usize - 1) } else { None };
            read(
                cx,
                match b {
                    Some(b) => b,
                    None if slot == 0 => cx.var_lookup_skip_raw(sym, self.rho, shape),
                    None => cx.var_lookup_raw(sym, self.rho),
                },
            )
        }
    }

    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot> {
        // SAFETY: a live expression of the caller's code and its rooted
        // environment; the code its call caches is live.
        unsafe {
            match e.typ() {
                SYMSXP => symbol(cx, e, self.rho),
                // (`early::run`'s reads, through this scope: early::run
                // keeps its one caller)
                LANGSXP => crate::code::cached(cx, e.aux(), Profile::Bc).and_then(|c| pure(cx, c, self)),
                _ => None,
            }
        }
    }
}

/// An inlined call.
struct Frame<'a> {
    /// The shape its code's slot indices are for: its formals, then its
    /// locals.
    shape: Sexp,
    /// The closure's environment.
    enclos: Sexp,
    /// Where its arguments are evaluated.
    caller: &'a dyn Scope,
    /// Its slots, in the shape's order: the formals' arguments, then the
    /// locals.
    nslots: usize,
    args: [Arg; MAX_SLOTS],
    /// The depth its evaluation points count from (an activation's
    /// `depth`: the call's depth and one).
    depth: i32,
}

impl Frame<'_> {
    /// The slot of `sym` read with slot index `slot` for `shape`.
    fn formal(&self, sym: Sexp, slot: u16, shape: Sexp) -> Option<usize> {
        if slot != 0 && shape == self.shape {
            return Some(slot as usize - 1);
        }
        // SAFETY: the code's shape, a VECSXP of `nslots` symbols.
        unsafe {
            let syms = self.shape.std_data() as *const Sexp;
            (0..self.nslots).find(|&k| *syms.add(k) == sym)
        }
    }
}

impl Scope for Frame<'_> {
    fn read(&self, cx: &Cx, sym: Sexp, slot: u16, shape: Sexp) -> Option<Slot> {
        // SAFETY: a symbol constant of live code.
        if unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0 {
            return None;
        }
        match self.formal(sym, slot, shape) {
            // (an argument not forced yet: forcing it from here would be a
            // force nested in another, given up)
            Some(k) => match self.args[k] {
                Arg::Value(s) => Some(s),
                Arg::Expr(_) | Arg::Default(_) => None,
                // SAFETY: the closure's environment, live; nothing collects.
                Arg::Unbound => unsafe { read(cx, cx.var_lookup_raw(sym, self.enclos)) },
            },
            // SAFETY: the closure's environment, live; nothing collects.
            None => unsafe { read(cx, cx.var_lookup_raw(sym, self.enclos)) },
        }
    }

    fn eval(&self, cx: &Cx, e: Sexp) -> Option<Slot> {
        // SAFETY: as in `Real::eval`.
        unsafe {
            match e.typ() {
                SYMSXP => self.read(cx, e, 0, cx.nil().sexp()),
                LANGSXP => crate::code::cached(cx, e.aux(), Profile::Bc).and_then(|c| pure(cx, c, self)),
                _ => None,
            }
        }
    }
}

/// `early::run` over `scope`: compiled argument code that only loads
/// constants, reads variables ([`Scope::read`]) and does arithmetic on its
/// fast path, up to its `Ret`.
///
/// # Safety
/// `code` is a live code object; nothing collects while this runs.
unsafe fn pure<S: Scope + ?Sized>(cx: &Cx, code: Sexp, scope: &S) -> Option<Slot> {
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    if v.nregs > MAX_ARG_REGS || v.profile != Profile::Bc {
        return None;
    }
    let mut regs = [Slot::EMPTY; MAX_ARG_REGS];
    for pc in 0..v.nins {
        // SAFETY: `pc` is below the code's instruction count.
        let i = unsafe { v.at(pc) };
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            return None;
        }
        let r = |x: u16| {
            debug_assert!((x as usize) < MAX_ARG_REGS);
            // SAFETY: an operand register of the code, below its register
            // count and so below MAX_ARG_REGS (checked above).
            Some(unsafe { *regs.get_unchecked(x as usize) })
        };
        let w = match i.op {
            // SAFETY: constants of the live code object.
            Op::LdConst => Slot::object(unsafe { v.k(i.e) }),
            // SAFETY: as above; the compiler loads plain scalar constants
            // only.
            Op::LdScalar => unsafe { scalar(v.k(i.e)) },
            // SAFETY: a symbol constant of the live code object.
            Op::GetVar => scope.read(cx, unsafe { v.k(i.e) }, i.f, v.shape)?,
            Op::Arith => match ops::arith2(i.g as u8, num(cx, r(i.b)?)?, num(cx, r(i.c)?)?) {
                (n, false) => num_slot(n),
                (_, true) => return None,
            },
            Op::Arith1 => num_slot(ops::arith1(i.g as u8, num(cx, r(i.b)?)?)),
            Op::Clear | Op::ClearRange | Op::Nop => continue,
            Op::Ret if i.g != 2 => return r(i.b).filter(|x| *x != Slot::EMPTY),
            _ => return None,
        };
        debug_assert!((i.a as usize) < MAX_ARG_REGS);
        // SAFETY: as for `r`.
        unsafe { *regs.get_unchecked_mut(i.a as usize) = w };
    }
    None
}

/// A plain length-1 constant as an unboxed scalar (`LdScalar`).
///
/// # Safety
/// `k` is a double, integer or logical constant of length 1.
unsafe fn scalar(k: Sexp) -> Slot {
    // SAFETY: the caller's contract.
    unsafe {
        match k.typ() {
            REALSXP => Slot::real(*(k.std_data() as *const f64)),
            INTSXP => Slot::int(*(k.std_data() as *const i32)),
            _ => Slot::logical(*(k.std_data() as *const i32)),
        }
    }
}

/// The counts an inline run changes, put back when it gives up.
struct Saved {
    evalcount: i32,
    bc_evalcount: i32,
    depth: i32,
    visible: bool,
}

impl Saved {
    fn of(cx: &Cx) -> Saved {
        Saved {
            evalcount: cx.evalcount().get(),
            bc_evalcount: cx.bc_evalcount().get(),
            depth: cx.eval_depth(),
            visible: cx.visible(),
        }
    }

    fn restore(&self, cx: &mut Cx) {
        cx.evalcount().set(self.evalcount);
        cx.bc_evalcount().set(self.bc_evalcount);
        cx.set_eval_depth(self.depth);
        cx.set_visible(self.visible);
    }
}

/// The VM's call `call` of the closure `f` (in a register of `act`, whose
/// code is byte code: its constant arguments are values, its promises byte
/// code, as the counts here assume), run inline into register `dst` when
/// `f`'s code is a
/// [`candidate`]: true when it ran. False with nothing changed otherwise
/// (`f` is then marked [`extra::NO_INLINE`] if its code can never run
/// inline): the real call runs. The call's entry safepoint comes first in
/// a real call, so a safepoint with work to do gives up (the real call
/// serves it).
///
/// # Safety
/// `call` is a constant of `act`'s live code; `f` is live.
#[inline(never)]
pub(crate) unsafe fn site(cx: &mut Cx, act: &Act, dst: Reg, f: Sexp, call: Sexp, site: u16) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let Some(code) = inline_code(cx, f) else {
            // a closure whose attached byte code is judged not inlinable
            // (code the JIT has not compiled yet is not judged)
            if is_judged_no(cx, f) {
                f.set_extra(f.extra() | extra::NO_INLINE);
            }
            return false;
        };
        let caller = act.obj(R_CODE);
        let stamp = crate::code::stamp(code);
        if cx.safepoint_pending() || crate::args::no_inline_at(caller, site, stamp) {
            return false;
        }
        match self::call(cx, code, f, call, act.rho()) {
            Ok(x) => {
                // the caller's register: compiled code takes an unboxed
                // value there (`Act::unboxed`)
                act.set(dst, x);
                let st = crate::prims::state(cx);
                st.inline_runs.set(st.inline_runs.get() + 1);
                true
            }
            // (the site's arguments cannot be matched to the callee's
            // formals: never, for this callee)
            Err(Unmatched) => {
                crate::args::mark_no_inline_at(caller, site, stamp);
                false
            }
            Err(_) => false,
        }
    }
}

/// Whether `f`'s attached code is closure byte code that cannot run inline
/// (judged now if it was not yet).
///
/// # Safety
/// `f` is a live closure.
#[cold]
unsafe fn is_judged_no(cx: &Cx, f: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let c = f.aux();
        crate::code::is_closure_code(cx, c, Profile::Bc, f.car()) && !candidate(cx, c, f)
    }
}

/// Why an inline run gave up.
enum GiveUp {
    /// The call's arguments do not match the callee's formals as an inline
    /// run binds them: so for every call from this site.
    Unmatched,
    /// Anything else.
    Run,
}
use GiveUp::*;

/// The call `call` of the closure `op` (whose code `code` is a
/// [`candidate`]) from compiled code running in `rho`, run inline: its
/// value, with GNU R's counts and `R_Visible` as the real call leaves them,
/// or why not, with nothing changed (the real call then runs). Called with
/// the depth the call's evaluation point set, where a real call's entry
/// safepoint would have nothing to do.
///
/// # Safety
/// `code`, `op`, `call` and `rho` are live objects of this isolate, rooted
/// by the caller.
unsafe fn call(cx: &mut Cx, code: Sexp, op: Sexp, call: Sexp, rho: Sexp) -> Result<Slot, GiveUp> {
    let saved = Saved::of(cx);
    let caller = Real { rho };
    // SAFETY: the caller's contract.
    let r = unsafe { enter(cx, code, op, call, &caller, saved.depth, 0) };
    match r {
        Ok(x) => {
            // (the real call's exec restores the depth it began at)
            cx.set_eval_depth(saved.depth);
            Ok(x)
        }
        Err(g) => {
            saved.restore(cx);
            Err(g)
        }
    }
}

/// A call's entry, as `args::apply` and `vm::exec` make it, then its body:
/// the arguments matched (exact names, then positions), the activation's
/// depth checked, `bcEval`'s entry count.
///
/// # Safety
/// As for [`call`]; `caller` evaluates the call's argument expressions.
unsafe fn enter(
    cx: &mut Cx,
    code: Sexp,
    op: Sexp,
    call: Sexp,
    caller: &dyn Scope,
    at: i32,
    nest: u32,
) -> Result<Slot, GiveUp> {
    // SAFETY: the caller's contract.
    unsafe {
        let v = View::of(code);
        let nslots = v.shape.std_len() as usize;
        // (the verdict's bound, for this shape: code::reshape starts a
        // copy's verdict again)
        debug_assert!(nslots <= MAX_SLOTS);
        let depth = at + 1;
        let mut fr = Frame { shape: v.shape, enclos: op.tag(), caller, nslots, args: [Arg::Unbound; MAX_SLOTS], depth };
        if !match_args(cx, op, call, formals_of(code), &mut fr.args) {
            return Err(Unmatched);
        }
        // vm::exec: the activation's depth (eval_depth_at), then bcEval's
        // entry poll count
        if depth > cx.expressions() {
            return Err(Run);
        }
        let c = cx.bc_evalcount().get() + 1;
        if c > 1023 {
            return Err(Run);
        }
        cx.bc_evalcount().set(c);
        body(cx, &v, &mut fr, nest).ok_or(Run)
    }
}

/// Match the call's actuals to the closure's `n` formals (the shape's
/// order): exact names first, then positions, as `matchArgs` does when no
/// partial match is involved. False for anything else: a partial name, too
/// many or too few arguments, an actual [`actual`] refuses.
///
/// # Safety
/// `op` and `call` are live.
unsafe fn match_args(cx: &Cx, op: Sexp, call: Sexp, n: usize, args: &mut [Arg; MAX_SLOTS]) -> bool {
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        // the common case: every actual by position
        let nil = cx.nil().sexp();
        let mut el = call.cdr();
        let mut k = 0;
        while el != nil {
            if el.tag() != nil {
                return match_named(cx, op, call, n, args);
            }
            if k == n {
                return false;
            }
            let Some(a) = actual(cx, el.car()) else { return false };
            args[k] = a;
            k += 1;
            el = el.cdr();
        }
        // the formals after the actuals: their defaults
        let mut f = op.car();
        for _ in 0..k {
            f = f.cdr();
        }
        while k < n {
            let Some(d) = default(cx, f.car()) else { return false };
            args[k] = d;
            k += 1;
            f = f.cdr();
        }
        true
    }
}

/// A missing formal's argument: its default when that is a constant `eval`
/// returns as it is; None for none (a missing argument) or any other.
///
/// # Safety
/// `d` is a formal's default (or R_MissingArg), live.
#[inline(always)]
unsafe fn default(cx: &Cx, d: Sexp) -> Option<Arg> {
    if d == cx.missing_arg().sexp() {
        return None;
    }
    // SAFETY: the caller's contract.
    match unsafe { d.typ() } {
        NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => Some(Arg::Default(d)),
        _ => None,
    }
}

/// An actual's argument: a promise of a symbol or a call, or a constant's
/// value; None for `...`, an empty argument, a getter call's `*tmp*`, or an
/// expression a compiled call would not promise.
///
/// # Safety
/// `e` is live.
#[inline(always)]
unsafe fn actual(cx: &Cx, e: Sexp) -> Option<Arg> {
    let st = crate::prims::state(cx);
    if e == st.syms.dots || e == cx.missing_arg().sexp() || e == st.syms.tmpval {
        return None;
    }
    // SAFETY: the caller's contract.
    unsafe {
        match e.typ() {
            SYMSXP if e.gp() & robigo_rtcore::DDVAL == 0 => Some(Arg::Expr(e)),
            LANGSXP => Some(Arg::Expr(e)),
            NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => Some(Arg::Value(Slot::object(e))),
            _ => None,
        }
    }
}

/// [`match_args`] for a call with a named actual.
///
/// # Safety
/// As for [`match_args`].
#[cold]
#[inline(never)]
unsafe fn match_named(cx: &Cx, op: Sexp, call: Sexp, n: usize, args: &mut [Arg; MAX_SLOTS]) -> bool {
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let nil = cx.nil().sexp();
        let mut set = [false; MAX_FORMALS];
        let mut positional = [Arg::Value(Slot::EMPTY); MAX_FORMALS];
        let mut npos = 0;
        let mut el = call.cdr();
        while el != nil {
            let (e, tag) = (el.car(), el.tag());
            let Some(arg) = actual(cx, e) else { return false };
            if tag != nil {
                let mut f = op.car();
                let mut k = 0;
                while k < n && f.tag() != tag {
                    f = f.cdr();
                    k += 1;
                }
                if k == n || set[k] {
                    return false;
                }
                set[k] = true;
                args[k] = arg;
            } else {
                if npos == MAX_FORMALS {
                    return false;
                }
                positional[npos] = arg;
                npos += 1;
            }
            el = el.cdr();
        }
        let mut p = 0;
        let mut f = op.car();
        for k in 0..n {
            if !set[k] {
                if p < npos {
                    args[k] = positional[p];
                    p += 1;
                } else {
                    // a formal without an argument: its default
                    let Some(d) = default(cx, f.car()) else { return false };
                    args[k] = d;
                }
            }
            f = f.cdr();
        }
        p == npos
    }
}

/// Run an inlined body.
///
/// # Safety
/// `v` is the view of the frame's code, live closure code that qualified
/// ([`candidate`]): at most [`MAX_REGS`] registers, which its instructions'
/// operands index.
unsafe fn body(cx: &mut Cx, v: &View, fr: &mut Frame<'_>, nest: u32) -> Option<Slot> {
    let mut regs = [Slot::EMPTY; MAX_REGS];
    let slack = cx.expressions() - fr.depth;
    let mut pc = 0;
    loop {
        // SAFETY: the compiler ends every path in Ret or Return, and its
        // jump targets are instructions of the code.
        let i = unsafe { v.at(pc) };
        pc += 1;
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            // (eval's count, polled past 1000: given up before the poll)
            if i.fl & F_TICK != 0 {
                let c = cx.evalcount().get() + 1;
                if c > 1000 {
                    return None;
                }
                cx.evalcount().set(c);
            }
            if i.fl & F_DEPTH != 0 && i.d as i32 > slack {
                return None;
            }
        }
        let vis = i.fl & F_VIS != 0;
        let r = |x: u16| {
            debug_assert!((x as usize) < MAX_REGS);
            // SAFETY: an operand register of the code, below its register
            // count and so below MAX_REGS (the caller's contract).
            let s = unsafe { *regs.get_unchecked(x as usize) };
            (s != Slot::EMPTY).then_some(s)
        };
        let w = match i.op {
            Op::Nop => continue,
            // SAFETY: constants of the live code object.
            Op::LdConst => Slot::object(unsafe { v.k(i.e) }),
            // SAFETY: as above; the compiler loads plain scalar constants
            // only.
            Op::LdScalar => unsafe { scalar(v.k(i.e)) },
            Op::GetVar => {
                if vis {
                    cx.set_visible(true);
                }
                // SAFETY: a symbol constant.
                let sym = unsafe { v.k(i.e) };
                match fr.formal(sym, i.f, v.shape) {
                    Some(k) => match fr.args[k] {
                        Arg::Value(s) => s,
                        Arg::Default(d) => {
                            // the force of a default: its entry safepoint,
                            // then `eval` of the constant (its count,
                            // polled past 1000, and R_Visible)
                            if cx.safepoint_pending() {
                                return None;
                            }
                            let c = cx.evalcount().get() + 1;
                            if c > 1000 {
                                return None;
                            }
                            cx.evalcount().set(c);
                            cx.set_visible(true);
                            let x = Slot::object(d);
                            fr.args[k] = Arg::Value(x);
                            x
                        }
                        Arg::Expr(e) => {
                            // the force, at the reading point's depth
                            cx.set_eval_depth(fr.depth + i.d as i32);
                            // SAFETY: a live expression of the caller's code.
                            if !early_force(cx, true, unsafe { e.typ() } == LANGSXP) {
                                return None;
                            }
                            let x = fr.caller.eval(cx, e)?;
                            fr.args[k] = Arg::Value(x);
                            x
                        }
                        // a local not assigned yet: the frame does not bind
                        // it
                        // SAFETY: the closure's environment; nothing
                        // collects.
                        Arg::Unbound => unsafe { read(cx, cx.var_lookup_raw(sym, fr.enclos))? },
                    },
                    // SAFETY: the closure's environment; nothing collects.
                    None => unsafe { read(cx, cx.var_lookup_raw(sym, fr.enclos))? },
                }
            }
            Op::SetVar => {
                // `defineVar` into the frame's slot (the VM's `set_var`: a
                // scalar stays unboxed there), then R_Visible off
                fr.args[i.f as usize - 1] = Arg::Value(r(i.a)?);
                cx.set_visible(false);
                continue;
            }
            Op::Jmp => {
                pc = i.target();
                continue;
            }
            Op::BrFalse => {
                let c = match num(cx, r(i.b)?)? {
                    ops::Num::Lgl(x) if x != ops::NA_LOGICAL => x != 0,
                    ops::Num::Int(x) if x != i32::MIN => x != 0,
                    ops::Num::Real(x) if !x.is_nan() => x != 0.0,
                    // (asLogicalNoNA's error)
                    _ => return None,
                };
                if !c {
                    pc = i.target();
                }
                continue;
            }
            Op::Guard => {
                // SAFETY: constants of the code; the enclosure is live. The
                // frame binds the formals only, and the guard's symbol is
                // not one ([`qualifies`]).
                let ok = unsafe { cx.fun_lookup_raw(v.k(i.e), fr.enclos) == Some(v.k(i.c)) };
                if !ok {
                    pc = i.target();
                }
                continue;
            }
            // SAFETY: as above; `f` 1: the shape lacks the name.
            Op::GetFun => Slot::object(unsafe { cx.fun_lookup_cached_raw(v.k(i.e), fr.enclos)? }),
            Op::Call => {
                // (the callee's entry safepoints: nothing allocated here, so
                // a collection is due only if one was before)
                if nest + 1 >= MAX_NEST || cx.safepoint_pending() {
                    return None;
                }
                let f = r(i.b)?.as_object()?;
                // SAFETY: a register's object is live; the call a constant.
                unsafe {
                    if f.typ() != CLOSXP || f.extra() & extra::NO_INLINE != 0 {
                        return None;
                    }
                    let code = inline_code(cx, f)?;
                    // the call's evaluation point sets the depth the
                    // callee's entry counts from
                    enter(cx, code, f, v.k(i.e), fr, fr.depth + i.d as i32, nest + 1).ok()?
                }
            }
            Op::Arith => match ops::arith2(i.g as u8, num(cx, r(i.b)?)?, num(cx, r(i.c)?)?) {
                (n, false) => num_slot(n),
                (_, true) => return None,
            },
            Op::Arith1 => num_slot(ops::arith1(i.g as u8, num(cx, r(i.b)?)?)),
            Op::Relop => {
                let (x, y) = (r(i.b)?, r(i.c)?);
                match (num(cx, x), num(cx, y)) {
                    (Some(x), Some(y)) => Slot::logical(ops::relop2(i.g as u8, x, y)),
                    _ if matches!(i.g as u8, EQOP | NEOP) => {
                        let (a, b) = (x.as_object()?, y.as_object()?);
                        // SAFETY: registers' objects are live.
                        let e = unsafe { ops::str_eq(cx, Ref::from_sexp(a, cx), Ref::from_sexp(b, cx))? };
                        Slot::logical(match e {
                            None => ops::NA_LOGICAL,
                            Some(eq) => (eq == (i.g as u8 == EQOP)) as i32,
                        })
                    }
                    _ => return None,
                }
            }
            Op::Not => Slot::logical(ops::not1(num(cx, r(i.b)?)?)),
            Op::Logic => Slot::logical(ops::logic2(i.g, num(cx, r(i.b)?)?, num(cx, r(i.c)?)?)),
            Op::AndOr1 | Op::AndOr2 => {
                // logical operands only (others are do_logic2's to check)
                let x = match r(i.b)? {
                    s if s.tag() == Tag::Logical => s.as_int()?,
                    _ => return None,
                };
                if i.op == Op::AndOr1 {
                    let decided = if i.c == 1 { x == 0 } else { x == 1 };
                    if decided {
                        pc = i.target();
                    }
                    Slot::logical(x)
                } else {
                    let x1 = r(i.a)?.as_int()?;
                    // R: logic.c:do_logic2
                    Slot::logical(if x1 != ops::NA_LOGICAL {
                        x
                    } else if i.c == 1 {
                        if x == ops::NA_LOGICAL || x != 0 { ops::NA_LOGICAL } else { x }
                    } else if x == ops::NA_LOGICAL || x == 0 {
                        ops::NA_LOGICAL
                    } else {
                        x
                    })
                }
            }
            Op::Return => return r(i.b),
            Op::Ret => {
                let x = r(i.b)?;
                match i.g {
                    1 => cx.set_visible(true),
                    2 => cx.set_visible(false),
                    _ => {}
                }
                return Some(x);
            }
            Op::Clear => Slot::EMPTY,
            Op::ClearRange => {
                for k in i.a..i.b {
                    *regs.get_mut(k as usize)? = Slot::EMPTY;
                }
                continue;
            }
            Op::Move => r(i.b)?,
            Op::SetVis => {
                cx.set_visible(i.g != 0);
                continue;
            }
            _ => return None,
        };
        // (the ops whose F_VIS the VM honours after their value; GetVar's
        // is set before its read)
        if vis && matches!(i.op, Op::LdConst | Op::Arith | Op::Arith1 | Op::Relop | Op::Not | Op::Logic) {
            cx.set_visible(true);
        }
        debug_assert!((i.a as usize) < MAX_REGS);
        // SAFETY: as for `r`.
        unsafe { *regs.get_unchecked_mut(i.a as usize) = w };
    }
}

/// The code a call of the closure `f` would run, if it runs the compiled
/// code attached as `call::closure_code`'s common case finds it and that
/// code is a [`candidate`].
///
/// # Safety
/// `f` is a live closure.
#[inline(always)]
pub(crate) unsafe fn inline_code(cx: &Cx, f: Sexp) -> Option<Sexp> {
    let st = crate::prims::state(cx);
    // SAFETY: the caller's contract: its 4th word is R_NilValue or a code
    // object.
    unsafe {
        let c = f.aux();
        // (not debugged, traced or stepped: GNU R's bits 0-3)
        (st.inline.get()
            && st.forced.is_none()
            && !st.jit.disable_bytecode
            && f.extra() & 0x000F == 0
            && crate::jit::compiled(f)
            && crate::code::is_closure_code(cx, c, Profile::Bc, f.car())
            && candidate(cx, c, f))
        .then_some(c)
    }
}
