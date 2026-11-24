//! Arguments a caller computes early (docs/g3.md §6.2's precomputed
//! arguments; perf-vm's caller-forced arguments, m1-plan P1).
//!
//! GNU R evaluates an argument when the callee first reads it. When the
//! expression runs no code and reads only bindings (a variable bound to a
//! value, or compiled arithmetic of variables and constants on its fast
//! path), its value cannot change before that read unless something else
//! happens first: R code runs, a builtin runs, a binding outside the
//! callee's frame is written. So the call computes it and binds the
//! argument delayed with that value ([`robigo_rtcore::SlotInit::Delayed`]),
//! and the isolate keeps the frame as its early frame. The first such event
//! drops every early value it holds ([`Cx::note_effect`]: closure calls in
//! `args::apply`, builtins in `Builtin::invoke`, writes in the environment
//! layer). A value still there at the callee's read is the value GNU R's
//! evaluation would give then; the force takes it and makes the counts the
//! evaluation would ([`early_force`]): `R_Visible`, the depth, `eval`'s or
//! `bcEval`'s evaluation count, and none of that when the evaluation would
//! poll or raise the depth error (it then evaluates as usual). Nothing is
//! evaluated twice where it is observable: the early evaluation itself is
//! pure, and a dropped value is evaluated at the force as GNU R does.

#![allow(unsafe_code)]

use crate::code::View;
use crate::ir::{F_CLR_B, F_CLR_C, F_DEPTH, F_NAMES, F_TICK, Ins, Op, Profile, SPECIAL_INTERNAL};
use crate::ops;
use crate::vm::num_slot;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RResult, RawBinding, Ref, Root, Slot};

/// The most registers a code object computed early may use (its
/// registers are a local array: nothing allocates while it runs).
pub(crate) const MAX_REGS: usize = 12;

/// The value of the argument expression `expr` (an actual of compiled
/// code, so its promise would hold byte code) in `rho`, if computing it runs
/// no code and has no effect: a symbol bound to a value, or a call whose
/// byte code, compiled by an earlier force ([`crate::eval::expr_code`]'s
/// cache; this compiles nothing), only loads constants, reads such
/// variables and does arithmetic on its fast path; or a translated call's
/// promise code (a BCODESXP, `Op::CallCodes`'s arguments) whose translation
/// an earlier force made ([`crate::compile::bytecode_code`]'s cache; this
/// translates nothing, and a translation cache's stub is no translation)
/// and passes the same check. `Ok(None)` when it is not such an expression.
/// Allocates the boxed result.
pub(crate) fn value<'s>(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'s>,
    expr: Root<'s>,
    rho: Root<'_>,
) -> RResult<Option<Root<'s>>> {
    let e = expr.bind(cx).sexp();
    // SAFETY: a rooted expression.
    let slot = match unsafe { e.typ() } {
        // SAFETY: a symbol is immortal; the environment rooted.
        SYMSXP => match unsafe { symbol(cx, e, rho.bind(cx).sexp()) } {
            Some(x) => x,
            None => return Ok(None),
        },
        LANGSXP => {
            // SAFETY: the rooted call's 4th word is R_NilValue or its code
            // cache; the code object it holds is live, the environment
            // rooted, and nothing collects before the run ends.
            match unsafe { crate::code::cached(cx, e.aux(), Profile::Bc).and_then(|c| run(cx, c, rho.bind(cx).sexp())) }
            {
                Some(x) => x,
                None => return Ok(None),
            }
        }
        // byte code itself (a translated call's promise code): out of line,
        // so that `value` is the code it was for a symbol and a call (its
        // arm inlined here moved the call arm's code: binarytrees +0.1%)
        BCODESXP => match bytecode_value(cx, expr, rho) {
            Some(x) => x,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    let x = cx.retry_alloc(|cx| crate::vm::box_slot(cx, slot))?;
    // SAFETY: an object of this isolate (fresh, or a binding's value),
    // rooted at once.
    Ok(Some(s.root(cx, unsafe { Ref::from_sexp(x, cx) })))
}

/// The value of the byte code `expr` (eval.rs's BCODESXP arm) computed
/// early, as [`value`]'s: its translation run by [`run`], when an earlier
/// force made it (a translation cache's stub is none) and it is early-pure.
/// The cheap refusals first, before any variable is read: no translation,
/// or code [`crate::code::early_pure`] does not mark (translated base code
/// passes such arguments on every call); the arm's own conditions
/// ([`bytecode_runs`]) only for a value the run found.
#[inline(never)]
fn bytecode_value(cx: &Cx, expr: Root<'_>, rho: Root<'_>) -> Option<Slot> {
    let e = expr.bind(cx).sexp();
    // SAFETY: the rooted byte code's 4th word is R_NilValue, its
    // translation's code object, or a translation cache's stub (no code
    // object: None).
    let c = unsafe { crate::code::cached(cx, e.aux(), Profile::Bc) }?;
    // SAFETY: a live code object.
    if !unsafe { crate::code::early_pure(c) } {
        return None;
    }
    // SAFETY: the code is live, the environment rooted, and nothing
    // collects before the run ends.
    let x = unsafe { run(cx, c, rho.bind(cx).sexp()) }?;
    // (the arm's own conditions last: they matter only for a value, and a
    // run declines at its first read of a formal not yet forced, base's
    // commonest argument: lapply +0.25% with them first)
    bytecode_runs(cx, expr).then_some(x)
}

/// Whether the byte code `expr`'s early value, just declined
/// ([`bytecode_value`]), is declined for good at its call site: its
/// translation exists and either is not early-pure, or its run declined (a
/// read of a variable not plainly bound: translated base code passes its
/// formals on, unforced, on every call). Not when there is no translation
/// yet (an earlier force makes it: the next call may take the value) or
/// the arm's own conditions refused (they change with the session).
#[cold]
#[inline(never)]
pub(crate) fn declined_for_good(cx: &Cx, expr: Root<'_>) -> bool {
    // SAFETY: as in bytecode_value.
    let Some(c) = (unsafe { crate::code::cached(cx, expr.bind(cx).sexp().aux(), Profile::Bc) }) else {
        return false;
    };
    // SAFETY: a live code object.
    !unsafe { crate::code::early_pure(c) } || bytecode_runs(cx, expr)
}

/// Whether eval.rs's BCODESXP arm would run the byte code `expr` (and so
/// whether its early value is the one a force would make): not when the
/// `Eval` profile is forced, byte code disabled, or its version is not one
/// bcEval runs (the arm evaluates the source then); nor with early byte
/// code turned off ([`crate::set_early_bytecode`], for tests). Inlined into
/// [`bytecode_value`], its one caller.
#[inline]
fn bytecode_runs(cx: &Cx, expr: Root<'_>) -> bool {
    let st = crate::prims::state(cx);
    st.early_bytecode.get()
        && st.forced != Some(Profile::Eval)
        && !st.jit.get().disable_bytecode
        && robigo_rtbuiltins::bytecode::version_ok(expr.bind(cx))
}

/// A variable's value read without running code: bound to a value (not
/// `R_MissingArg`, nor `...`'s list), an evaluated promise or forced
/// argument, or an unboxed scalar. None for anything a read would have to
/// evaluate or check (`..N`, an active binding, an unforced promise).
///
/// # Safety
/// `sym` is a symbol and `rho` a live environment; the value is live until
/// the next safepoint.
pub(crate) unsafe fn symbol(cx: &Cx, sym: Sexp, rho: Sexp) -> Option<Slot> {
    // SAFETY: the caller's contract.
    unsafe {
        if sym.gp() & robigo_rtcore::DDVAL != 0 {
            return None;
        }
        read(cx, cx.var_lookup_raw(sym, rho))
    }
}

/// A binding's value as `get_var_fast` reads it, or None.
///
/// # Safety
/// `b` was just found; its objects are live.
pub(crate) unsafe fn read(cx: &Cx, b: RawBinding) -> Option<Slot> {
    // SAFETY: the caller's contract.
    unsafe {
        let v = match b {
            RawBinding::Value(v) => v,
            RawBinding::Forced(v) => return Some(Slot::object(v)),
            RawBinding::Scalar(x) => {
                return Some(match x {
                    robigo_rtcore::Scalar::Real(r) => Slot::real(r),
                    robigo_rtcore::Scalar::Int(i) => Slot::int(i),
                    robigo_rtcore::Scalar::Lgl(l) => Slot::logical(l.0),
                });
            }
            _ => return None,
        };
        let v = if v.typ() == PROMSXP {
            let pv = v.car();
            if pv == cx.unbound().sexp() || v.promise_tag() != 0 {
                return None;
            }
            pv
        } else {
            // (`...` evaluated is an error; its list is a DOTSXP)
            if v == cx.missing_arg().sexp() || v == cx.unbound().sexp() || v.typ() == DOTSXP {
                return None;
            }
            v
        };
        Some(Slot::object(v))
    }
}

/// D9's deaths of instruction `i` (at `at`) in an evaluator's own
/// registers `regs`, as `vm::run` makes them in an activation's, under
/// `ROBIGO_POISON_DEAD` only (C1-PZ): its operands read for the last time
/// (`F_CLR_B`, `F_CLR_C`, not its destination), a builtin call's or a
/// default subset's register window, and `Clear`'s and `ClearRange`'s
/// registers. The evaluators that run compiled code outside
/// the VM ([`run`], `inline::pure`, `inline::body_loop`) apply it to the
/// instruction they ran, before the next one reads, so a register read
/// after its last use panics there too.
#[inline(always)]
pub fn poison_deaths(regs: &mut [Slot], i: &Ins, at: usize) {
    if !robigo_rtcore::poison_dead() {
        return;
    }
    let mut kill = |r: u16| {
        if let Some(s) = regs.get_mut(r as usize) {
            *s = Slot::dead(r, at as u32);
        }
    };
    // a register-window operation's window, `n` arguments from `b` (and
    // their names after them), dead after the call (`call_builtin_regs`,
    // `dflt_subset` and its fast path's `clear_subset_operands`)
    let window = |i: &Ins| i.b..i.b + if i.fl & F_NAMES != 0 { 2 * i.c } else { i.c };
    match i.op {
        Op::Clear => kill(i.a),
        Op::ClearRange => (i.a..i.b).for_each(kill),
        Op::CallBuiltin | Op::Internal if i.c != SPECIAL_INTERNAL && i.c != u16::MAX => window(i).for_each(kill),
        Op::DfltSubset => window(i).for_each(kill),
        _ => {
            if i.fl & F_CLR_B != 0 && i.b != i.a {
                kill(i.b);
            }
            if i.fl & F_CLR_C != 0 && i.c != i.a {
                kill(i.c);
            }
        }
    }
}

/// Run `code` in `rho` if every instruction is one that runs no code and
/// has no effect: constant loads, variable reads ([`read`]) and arithmetic
/// on its fast path (no warning), up to the code's `Ret`. None at anything
/// else (an evaluation point that counts, a call, a branch, a slow path).
///
/// # Safety
/// `code` is a live code object and `rho` a live environment; nothing may
/// collect while this runs (it does not allocate).
// (inlined into `value` and into `bytecode_value`: called from two places
// it was not, +0.49% on binarytrees, step 1a)
#[inline(always)]
unsafe fn run(cx: &Cx, code: Sexp, rho: Sexp) -> Option<Slot> {
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    if v.nregs > MAX_REGS || v.profile != Profile::Bc {
        return None;
    }
    let mut regs = [Slot::EMPTY; MAX_REGS];
    for pc in 0..v.nins {
        if pc > 0 {
            // SAFETY: as below, the instruction just run.
            poison_deaths(&mut regs, &unsafe { *v.ins.add(pc - 1) }, pc - 1);
        }
        // SAFETY: `pc` is below the code's instruction count.
        let i: Ins = unsafe { *v.ins.add(pc) };
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            return None;
        }
        let r = |x: u16| {
            let s = regs.get(x as usize).copied();
            if let Some(s) = s {
                s.check_live();
            }
            s
        };
        let w = match i.op {
            // SAFETY: a constant of the live code object.
            Op::LdConst => unsafe { Slot::object(v.k(i.e)) },
            Op::LdScalar => {
                // SAFETY: as above; the compiler loads plain scalar
                // constants only.
                unsafe {
                    let k = v.k(i.e);
                    match k.typ() {
                        REALSXP => Slot::real(*(k.std_data() as *const f64)),
                        INTSXP => Slot::int(*(k.std_data() as *const i32)),
                        _ => Slot::logical(*(k.std_data() as *const i32)),
                    }
                }
            }
            Op::GetVar => {
                // SAFETY: the symbol is a constant of the live code; the
                // environment is live.
                unsafe {
                    let sym = v.k(i.e);
                    if sym.gp() & robigo_rtcore::DDVAL != 0 {
                        return None;
                    }
                    // the slot the index names in a frame of the code's
                    // index shape; a name the shape lacks from the
                    // enclosure of such a frame (the VM's GetVar)
                    let b = if i.f != 0 { cx.frame_slot_raw(rho, v.shape, i.f as usize - 1) } else { None };
                    read(
                        cx,
                        match b {
                            Some(b) => b,
                            None if i.f == 0 => cx.var_lookup_skip_raw(sym, rho, v.shape),
                            None => cx.var_lookup_raw(sym, rho),
                        },
                    )?
                }
            }
            Op::Arith => {
                // SAFETY: `num` only reads: nothing collects while `run` runs (its
                // contract), so the operands' objects stay live (the registers are a
                // local array, not rooted).
                let (x, y) = (unsafe { crate::vm::num(cx, r(i.b)?) }?, unsafe { crate::vm::num(cx, r(i.c)?) }?);
                // (byte code: `run` takes no other)
                match ops::arith2(i.g as u8, x, y, true) {
                    (n, false) => num_slot(n),
                    // (an overflow's or a lost modulus's warning)
                    (_, true) => return None,
                }
            }
            // SAFETY: `num` only reads: nothing collects while `run` runs (its
            // contract), so the operands' objects stay live (the registers are a
            // local array, not rooted).
            Op::Arith1 => num_slot(ops::arith1(i.g as u8, unsafe { crate::vm::num(cx, r(i.b)?) }?)),
            Op::Clear | Op::ClearRange | Op::Nop => continue,
            // the code's value, visible (`Ret` 2 would make it invisible)
            Op::Ret if i.g != 2 => return r(i.b).filter(|x| *x != Slot::EMPTY),
            _ => return None,
        };
        *regs.get_mut(i.a as usize)? = w;
    }
    None
}

/// [`robigo_rtcore::hooks::EarlyForceFn`]: the counts the evaluation of a
/// delayed argument's expression (of SEXPTYPE `kind`) makes in
/// `eval::eval_in` (the path a force without an early value takes), made in
/// place, with `R_Visible` and the depth check:
/// - a symbol: `eval`'s evaluation count;
/// - a call: its code's `vm::exec` entry, `bcEval`'s entry poll count;
/// - byte code (a translated call's promise code): both, `eval`'s count
///   (eval.rs's `tick` before the BCODESXP arm) and then `bcEval`'s entry
///   poll (`vm::exec`, or `run_single_read`'s).
///
/// False, with nothing counted, when any count would poll or the depth
/// would pass `R_Expressions`.
// R: eval.c:eval, eval.c:bcEval (the entry's BC_CHECK_SIGINT)
pub fn early_force(cx: &mut Cx, bytecode: bool, kind: u8) -> bool {
    // a call's argument takes one type test, as before byte-code objects
    // came here (rtcore's caller tested for a call itself then): the
    // byte-code test only off the call's path
    let call = kind == LANGSXP;
    if !call && kind == BCODESXP {
        // (a tail call to a cold function: the common path keeps no frame)
        return early_force_bc(cx, bytecode);
    }
    early_force_ast(cx, bytecode, call)
}

/// [`early_force`] for a symbol (`call` false) or a call: one count each,
/// `eval`'s or `bcEval`'s entry poll's. Inlined into the inliner's `enter`,
/// whose arguments are never byte-code objects (a translated call's
/// `CallCodes` does not reach `inline::site`): the byte-code case kept out
/// of it, as an extra compare there cost 7 instructions a call (step 1a).
/// (No inline attribute, as the one function before it had none: forcing
/// it inline shifted the inliner's own inlining, +0.1% on binarytrees.)
pub(crate) fn early_force_ast(cx: &mut Cx, bytecode: bool, call: bool) -> bool {
    // (only compiled code's arguments are computed early: their promises
    // hold byte code, evaluated under the `Bc` profile)
    if !bytecode {
        return false;
    }
    if cx.eval_depth() + 1 > cx.expressions() {
        return false;
    }
    let count = if call { cx.bc_evalcount() } else { cx.evalcount() };
    let n = count.get() + 1;
    if n > if call { 1023 } else { 1000 } {
        return false;
    }
    count.set(n);
    cx.set_visible(true);
    true
}

/// [`early_force`] for a byte-code object (a translated call's promise
/// code, byte code's slot): `eval`'s count and then `bcEval`'s entry poll
/// count, both made or neither.
#[cold]
#[inline(never)]
fn early_force_bc(cx: &mut Cx, bytecode: bool) -> bool {
    if !bytecode || cx.eval_depth() + 1 > cx.expressions() {
        return false;
    }
    let (e, b) = (cx.evalcount().get() + 1, cx.bc_evalcount().get() + 1);
    if e > 1000 || b > 1023 {
        return false;
    }
    cx.evalcount().set(e);
    cx.bc_evalcount().set(b);
    cx.set_visible(true);
    true
}
