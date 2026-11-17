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
use crate::ir::{F_DEPTH, F_TICK, Ins, Op, Profile};
use crate::ops;
use crate::vm::num_slot;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RResult, RawBinding, Ref, Root, Slot};

/// The most registers a code object computed early may use (its
/// registers are a local array: nothing allocates while it runs).
const MAX_REGS: usize = 12;

/// The value of the argument expression `expr` (an actual of compiled
/// code, so its promise would hold byte code) in `rho`, if computing it runs
/// no code and has no effect: a symbol bound to a value, or a call whose
/// byte code, compiled by an earlier force ([`crate::eval::expr_code`]'s
/// cache; this compiles nothing), only loads constants, reads such
/// variables and does arithmetic on its fast path. `Ok(None)` when it is not
/// such an expression. Allocates the boxed result.
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
        _ => return Ok(None),
    };
    let x = cx.retry_alloc(|cx| crate::vm::box_slot(cx, slot))?;
    // SAFETY: an object of this isolate (fresh, or a binding's value),
    // rooted at once.
    Ok(Some(s.root(cx, unsafe { Ref::from_sexp(x, cx) })))
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

/// Run `code` in `rho` if every instruction is one that runs no code and
/// has no effect: constant loads, variable reads ([`read`]) and arithmetic
/// on its fast path (no warning), up to the code's `Ret`. None at anything
/// else (an evaluation point that counts, a call, a branch, a slow path).
///
/// # Safety
/// `code` is a live code object and `rho` a live environment; nothing may
/// collect while this runs (it does not allocate).
unsafe fn run(cx: &Cx, code: Sexp, rho: Sexp) -> Option<Slot> {
    // SAFETY: the caller's contract.
    let v = unsafe { View::of(code) };
    if v.nregs > MAX_REGS || v.profile != Profile::Bc {
        return None;
    }
    let mut regs = [Slot::EMPTY; MAX_REGS];
    for pc in 0..v.nins {
        // SAFETY: `pc` is below the code's instruction count.
        let i: Ins = unsafe { *v.ins.add(pc) };
        if i.fl & (F_TICK | F_DEPTH) != 0 {
            return None;
        }
        let r = |x: u16| regs.get(x as usize).copied();
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
                // SAFETY: registers of the running activation: their objects are live, rooted.
                let (x, y) = (unsafe { crate::vm::num(cx, r(i.b)?) }?, unsafe { crate::vm::num(cx, r(i.c)?) }?);
                // (byte code: `run` takes no other)
                match ops::arith2(i.g as u8, x, y, true) {
                    (n, false) => num_slot(n),
                    // (an overflow's or a lost modulus's warning)
                    (_, true) => return None,
                }
            }
            // SAFETY: registers of the running activation: their objects are live, rooted.
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
/// delayed argument's expression makes in `eval::eval_bytecode` (the path
/// a force without an early value takes), made in place: for a symbol
/// `eval`'s `R_Visible`, evaluation count and depth check; for a call the
/// code's `vm::exec` entry: `R_Visible`, the depth check and `bcEval`'s
/// entry poll count. False, with nothing counted, when either count would
/// poll or the depth would pass `R_Expressions`.
// R: eval.c:eval, eval.c:bcEval (the entry's BC_CHECK_SIGINT)
pub fn early_force(cx: &mut Cx, bytecode: bool, call: bool) -> bool {
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
