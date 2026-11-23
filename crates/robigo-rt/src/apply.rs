//! Applying a function to a call object (GNU R's `eval` of a LANGSXP once
//! the function is found, and `R_forceAndCall`'s): a special applied to its
//! call, and a call made at run time evaluated without compiling it
//! (complex assignment's getter and setter calls). A closure goes through
//! [`crate::call::apply_closure`] (C1's seam), a builtin through `evalList`
//! ([`crate::call::builtin_evallist`]).

#![allow(unsafe_code)]

use crate::call::{builtin_evallist, call_special};
use crate::compile;
use crate::ir::Reg;
use crate::prims::{self, Kind};
use crate::vm::{self, Act};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Out, RResult, Ref, Root, Slot, ctxt};
use std::ptr::NonNull;

/// The special `fun` (a function over its call) for the call `call`, its
/// guard passed: [`crate::ir::Op::SpecialFn`].
///
/// # Safety
/// `fun` is a live SPECIALSXP and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
pub unsafe fn special_fn(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    // SAFETY: the caller's contract: `fun` and `call` are live.
    unsafe { call_special(cx, act, dst, fun, call) }
}

/// A special applied to its call in `rho` (`eval`'s SPECIALSXP case,
/// `R_forceAndCall`'s): a language special's call compiled (with the
/// special inlined) and run; a special that is a function over its call
/// run directly. `R_Visible` as `PRIMPRINT` says.
// R: eval.c:eval (SPECIALSXP)
pub fn apply_special(cx: &mut Cx, call: Root<'_>, fun: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    apply_special_args(cx, call, None, fun, rho, out)
}

/// [`apply_special`] with the arguments `args` (a pairlist) in place of the
/// call's: `applyMethod`'s special method, its arguments the method's
/// promises.
// R: objects.c:applyMethod (SPECIALSXP)
pub fn apply_special_args(
    cx: &mut Cx,
    call: Root<'_>,
    args: Option<Root<'_>>,
    fun: Root<'_>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // SAFETY: a rooted primitive of this isolate.
    let off = unsafe { prims::offset(fun.bind(cx).sexp()) };
    let def = prims::state(cx).defs[off];
    match def.kind {
        // PRIMFUN(op)(call, op, args, rho): `args` the method's promises
        // (`applyMethod`), `call` the method's `newcall`, which its messages
        // name; None for the call's own arguments
        Kind::Fn(f) => {
            let flag = def.print();
            // a builtin may change what an argument expression evaluates to or
            // look at an argument: values computed early are dropped (crate::early)
            cx.note_effect();
            cx.set_visible(flag != 1);
            f(cx, call, fun, args, rho, out)?;
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            Ok(())
        }
        Kind::Special(_) => cx.scope(|cx, s| {
            let code = compile::compile_special_with(cx, call, args, off)?;
            // SAFETY: fresh.
            let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
            let slot = NonNull::new(out.root().as_ptr() as *mut Slot).unwrap();
            // the code's root is the call's own `eval` (its operands are
            // compiled under eval's PROTECT(op) and PROTECT(e)), two
            // protections below the special's entry
            cx.with_pp_excess(-2, |cx| {
                // SAFETY: the code and environment rooted; the destination
                // is an empty slot.
                unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false, false) }.map(drop)
            })
        }),
        // a special of the builtin registry (`rep`): its function gets the
        // call's argument expressions (eval.c's SPECIALSXP case)
        Kind::Builtin(f, _) => cx.scope(|cx, s| {
            let (vals, names) = {
                let c = call.bind(cx);
                let (mut vals, mut names) = (Vec::new(), Vec::new());
                let mut a = match args {
                    Some(a) => a.bind(cx).as_cons(),
                    None => c.as_cons().and_then(|c| c.cdr().as_cons()),
                };
                while let Some(cell) = a {
                    vals.push(cell.car());
                    names.push(cell.tag().as_sym());
                    a = cell.cdr().as_cons();
                }
                (vals, names)
            };
            let named = names.iter().any(Option::is_some);
            let args = s.args(cx, &vals, named.then_some(&names[..]));
            crate::call::check_internal_arity(cx, &def, args.len())?;
            let flag = def.print();
            cx.note_effect();
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            f(cx, call, args, rho, out)?;
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            Ok(())
        }),
    }
}

/// The isolate's `force_and_call` hook: `R_forceAndCall(call, n, rho)`
/// (`lapply`'s and `vapply`'s `FUN(X[[i]], ...)` with `n` 1, `mapply`'s
/// call, `forceAndCall`), under its PROTECT(fun) and the arguments' or
/// the call's: the function is `findFun` of a symbol head, else the head
/// evaluated; a primitive is called as `eval` calls it but with no
/// `check_stack_balance`; a closure is applied to the call's promised
/// arguments, the first `n` forced before its body runs.
// R: eval.c:R_forceAndCall
pub fn force_and_call(cx: &mut Cx, call: Root<'_>, n: i32, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    force_and_call_with(cx, call, n, rho, None, out)
}

/// The isolate's `force_and_call_indexed` hook: [`force_and_call`] of base's
/// lapply/vapply call `FUN(X[[i]], ...)` with `n` 1, `x` the vector `X`
/// evaluated to and `ind` the one `i` binds, whose first argument's promise
/// may get its value directly ([`force_x_i`]).
pub fn force_and_call_indexed(
    cx: &mut Cx,
    call: Root<'_>,
    rho: Root<'_>,
    x: Root<'_>,
    ind: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    force_and_call_with(cx, call, 1, rho, Some((x, ind)), out)
}

fn force_and_call_with(
    cx: &mut Cx,
    call: Root<'_>,
    n: i32,
    rho: Root<'_>,
    direct: Option<(Root<'_>, Root<'_>)>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: the head of the rooted call.
        let head = s.root(cx, unsafe { Ref::from_sexp(call.bind(cx).sexp().car(), cx) });
        let f = s.out(cx);
        let fr = f.root();
        if head.bind(cx).sexptype() == SYMSXP {
            // findFun: an error if there is none
            // SAFETY: an immortal symbol and a rooted environment.
            match unsafe { cx.fun_lookup_raw(head.bind(cx).sexp(), rho.bind(cx).sexp()) } {
                // SAFETY: a function bound in a live frame.
                Some(fun) => f.set(cx, unsafe { Ref::from_sexp(fun, cx) }),
                None => cx.find_fun3(head, rho, None, f)?,
            }
        } else {
            crate::eval::eval(cx, head, rho, f)?;
        }
        // SAFETY: a rooted object.
        let t = unsafe { fr.bind(cx).sexp().typ() };
        cx.with_pp_excess(2, |cx| match t {
            SPECIALSXP => apply_special(cx, call, fr, rho, out),
            BUILTINSXP => builtin_evallist(cx, call, fr, rho, out, false, None),
            CLOSXP => {
                let pargs = s.out(cx).root();
                let o = pargs.reset(cx);
                crate::dispatch::promise_call_args(cx, call, rho, o)?;
                // the first n arguments forced: promises, else GNU R's errors
                let mut i = 0;
                while i < n {
                    // the i-th cell, read again after each forcing (which
                    // may collect)
                    let p = {
                        let mut a = pargs.bind(cx).as_cons();
                        for _ in 0..i {
                            a = a.and_then(|c| c.cdr().as_cons());
                        }
                        match a {
                            Some(c) => c.car().sexp(),
                            None => break,
                        }
                    };
                    // SAFETY: a live element of the rooted list (nothing
                    // collected since it was read).
                    if unsafe { p.typ() } == PROMSXP {
                        cx.scope(|cx, s2| {
                            // SAFETY: as above; rooted at once.
                            let pr = s2.root(cx, unsafe { Ref::from_sexp(p, cx) });
                            if i == 0
                                && let Some((x, ind)) = direct
                                && force_x_i(cx, s2, call, pr, rho, x, ind)?
                            {
                                return Ok(());
                            }
                            let o = s2.out(cx);
                            // R: eval(p, rho)
                            crate::eval::eval_promise(cx, pr, o)
                        })?;
                    } else if p == cx.missing_arg().sexp() {
                        let msg = format!("argument {} is empty", i + 1);
                        return Err(cx.error_call(Some(call), &msg));
                    } else {
                        return Err(cx.error("something weird happened"));
                    }
                    i += 1;
                }
                crate::call::apply_closure(cx, call, fr, pargs, rho, None, out)
            }
            _ => Err(cx.error("attempt to apply non-function")),
        })
    })
}

/// The levels below the caller the general force of `X[[i]]`'s promise
/// reaches: `eval` of the promise, of `X[[i]]`, and of `[[`'s argument `X`
/// (or `i`), each `INCREMENT_EVAL_DEPTH`.
const X_I_DEPTH: i32 = 3;

/// C stack a general force of `X[[i]]` may use, at least: below this
/// headroom the general path runs (its `R_CheckStack` raises where GNU R's
/// does).
const X_I_STACK: usize = 256 * 1024;

/// `forcePromise` of lapply's `X[[i]]` promise `p` with its value computed
/// directly, when GNU R's force of it is unobservable (true; false: the
/// caller forces it as usual, nothing done here that it would not do).
///
/// GNU R's `R_forceAndCall` forces the first argument before the closure's
/// frame is made: `eval(p)` → `forcePromise` → `eval(X[[i]])` →
/// `do_subset2` → `R_DispatchOrEvalSP`, which evaluates `X` and `i` and
/// dispatches on nothing for an unclassed `X` → `do_subset2_dflt`. Four
/// `eval`s: their evaluation-count ticks are taken here (an interrupt or a
/// finalizer runs at GNU R's points); their depth and C stack checks cannot
/// fail (headroom checked first, else the general path, which raises where
/// GNU R does). The guards, read after the ticks (a finalizer may run R
/// code): `p` is the call's own unforced `X[[i]]` in `rho`, `X` in `rho` is
/// still `x` with no class, `i` still `ind`, and `[[` from `rho` is base's
/// primitive. The value is `do_subset2_dflt`'s own; the promise is left as
/// `forcePromise` leaves it (PRVALUE, PRSEEN 0, PRENV R_NilValue; PRCODE the
/// call's `X[[i]]`, which `substitute` and `sys.call()` show).
// R: eval.c:R_forceAndCall, forcePromise, eval; subset.c:do_subset2
#[allow(clippy::too_many_arguments)]
fn force_x_i(
    cx: &mut Cx,
    s: robigo_rtcore::Scope<'_>,
    call: Root<'_>,
    p: Root<'_>,
    rho: Root<'_>,
    x: Root<'_>,
    ind: Root<'_>,
) -> RResult<bool> {
    // the promise is the call's X[[i]] (an AST one, unforced, in rho)
    let Some(tmp) = call.bind(cx).as_cons().and_then(|c| c.cdr().as_cons()).map(|c| c.car()) else { return Ok(false) };
    let pv = p.bind(cx);
    let plain = cx.promise_unforced_ast(pv)
        && match pv.view() {
            robigo_rtcore::View::Promise(pr) => pr.code() == tmp && pr.env() == rho.bind(cx),
            _ => false,
        };
    if !plain || cx.eval_depth() + X_I_DEPTH + 1 > cx.expressions() || !cx.c_stack_headroom(X_I_STACK) {
        return Ok(false);
    }
    // the four evaluations' ticks
    let before = cx.evalcount().get();
    let mut polled = false;
    for _ in 0..4 {
        polled |= crate::eval::tick(cx)?;
    }
    if !x_i_guards(cx, call, rho, x, ind) {
        if !polled {
            // (nothing ran: the general path ticks them itself)
            cx.evalcount().set(before);
        }
        return Ok(false);
    }
    let tmp = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().car());
    let v = s.out(cx);
    let vr = v.root();
    let args = s.args(cx, &[x.bind(cx), ind.bind(cx)], None);
    robigo_rtbuiltins::subset::do_subset2_dflt(cx, tmp, args, rho, v)?;
    cx.set_promise_forced(p, vr);
    Ok(true)
}

/// [`force_x_i`]'s guards: `X` in `rho` is `x` without a class, `i` is
/// `ind`, and `[[` from `rho` is base's primitive.
fn x_i_guards(cx: &Cx, call: Root<'_>, rho: Root<'_>, x: Root<'_>, ind: Root<'_>) -> bool {
    let tmp = call.bind(cx).as_cons().and_then(|c| c.cdr().as_cons()).map(|c| c.car());
    let Some(t) = tmp.and_then(|t| t.as_cons()) else { return false };
    let Some(t1) = t.cdr().as_cons() else { return false };
    let Some(t2) = t1.cdr().as_cons() else { return false };
    let (b2, xs, is) = (t.car().sexp(), t1.car().sexp(), t2.car().sexp());
    let (xv, rv) = (x.bind(cx), rho.bind(cx).sexp());
    if xv.is_object() {
        return false;
    }
    let value_of = |sym: Sexp| -> Option<Sexp> {
        // SAFETY: an immortal symbol and a rooted environment; read only.
        match unsafe { cx.var_lookup_raw(sym, rv) } {
            robigo_rtcore::RawBinding::Value(v) | robigo_rtcore::RawBinding::Forced(v) => {
                // a forced promise reads as its value
                // SAFETY: a live object read from a live frame.
                if unsafe { v.typ() } == PROMSXP {
                    match unsafe { Ref::from_sexp(v, cx) }.view() {
                        robigo_rtcore::View::Promise(pr) if !pr.is_unboxed() && pr.value() != cx.unbound() => {
                            Some(pr.value().sexp())
                        }
                        _ => None,
                    }
                } else {
                    Some(v)
                }
            }
            _ => None,
        }
    };
    if value_of(xs) != Some(xv.sexp()) || value_of(is) != Some(ind.bind(cx).sexp()) {
        return false;
    }
    // SAFETY: as above.
    match unsafe { cx.fun_lookup_raw(b2, rv) } {
        // SAFETY: a function bound in a live frame.
        Some(f) => prims::prim_name(cx, unsafe { Ref::from_sexp(f, cx) }) == Some("[["),
        None => false,
    }
}

/// `forceAndCall(n, FUN, ...)`: the call `FUN(...)` made of the rest of
/// the call, its first `n` arguments forced before a closure's body runs.
// R: eval.c:do_forceAndCall
pub fn do_force_and_call(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: run again on the call over it)
    if args.is_some() {
        return crate::specials::rerun_apart(cx, call, _op, args, rho, out, do_force_and_call);
    }
    cx.scope(|cx, s| {
        let (nexpr, rest) = {
            let args = call.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr());
            // CADR(call), CDDR(call): R_NilValue past the end
            let c = args.as_cons();
            (c.map_or(cx.nil(), |c| c.car()), c.map_or(cx.nil(), |c| c.cdr()))
        };
        let (nexpr, rest) = (s.root(cx, nexpr), s.root(cx, rest));
        let nv = s.capture(cx, |cx, o| crate::eval::eval(cx, nexpr, rho, o))?;
        let n = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, nv)?;
        // LCONS(CAR(e), CDR(e)): CDDR(call) as a call
        let e = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let r = rest.bind(cx);
                let (car, cdr) = r.as_cons().map_or((cx.nil(), cx.nil()), |c| (c.car(), c.cdr()));
                cx.lcons(car, cdr)
            })
        })?;
        force_and_call(cx, e, n, rho, out)
    })
}

/// `eval(call, rho)` for a call made at run time (complex assignment's
/// getter and setter calls): `eval`'s LANGSXP case, the function found
/// from `rho` and applied, without compiling the call (it is new each
/// time; its arguments are evaluated as compiled code).
// R: eval.c:eval (LANGSXP)
pub fn eval_call(cx: &mut Cx, call: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.set_visible(true);
    crate::eval::tick(cx)?;
    let depthsave = cx.eval_depth();
    cx.eval_depth_at(depthsave + 1)?;
    // the call's `eval` one level deeper with no protection more
    // (robigo_rtcore's ppacct)
    let r = cx.with_pp_excess(-1, |cx| {
        cx.scope(|cx, s| {
            let f = s.out(cx);
            let fr = f.root();
            // SAFETY: the head of the rooted call.
            let head = s.root(cx, unsafe { Ref::from_sexp(call.bind(cx).sexp().car(), cx) });
            if head.bind(cx).sexptype() == SYMSXP {
                // SAFETY: an immortal symbol and a rooted environment.
                match unsafe { cx.fun_lookup_raw(head.bind(cx).sexp(), rho.bind(cx).sexp()) } {
                    // SAFETY: a function bound in a live frame.
                    Some(fun) => f.set(cx, unsafe { Ref::from_sexp(fun, cx) }),
                    None => {
                        // "This picks the correct/better error expression for
                        // replacement calls running in the AST interpreter."
                        let g = cx.global_context();
                        let ecall = if g.callflag() == ctxt::CCODE { s.root(cx, g.call()) } else { call };
                        cx.find_fun3(head, rho, Some(ecall), f)?;
                    }
                }
            } else {
                crate::eval::eval(cx, head, rho, f)?;
            }
            // the function's entry: eval's PROTECT(op), then evalList's result
            // (a builtin), PROTECT(e) (a special) or the promised arguments (a
            // closure)
            cx.with_pp_excess(2, |cx| {
                // SAFETY: a rooted object.
                let t = unsafe { fr.bind(cx).sexp().typ() };
                if t == SPECIALSXP || t == BUILTINSXP {
                    // eval's check_stack_balance, `save` after PROTECT(op)
                    // SAFETY: a rooted primitive of this isolate.
                    let name = unsafe { prims::def(cx, fr.bind(cx).sexp()) }.name;
                    let save = cx.pp_save(-1);
                    if t == SPECIALSXP {
                        apply_special(cx, call, fr, rho, out)?;
                    } else {
                        builtin_evallist(cx, call, fr, rho, out, false, None)?;
                    }
                    cx.check_stack_balance(name.as_bytes(), save);
                    return Ok(());
                }
                match t {
                    // the arguments from the call in rho, as promiseArgs makes
                    // them; a call made at run time has no call site (no match
                    // plan)
                    CLOSXP => crate::args::apply(
                        cx,
                        call,
                        fr,
                        crate::args::Source::Site(crate::ir::NO_SITE, 0),
                        None,
                        rho,
                        None,
                        None,
                        true,
                        out,
                    ),
                    _ => Err(cx.error("attempt to apply non-function")),
                }
            })
        })
    });
    cx.set_eval_depth(depthsave);
    r
}
