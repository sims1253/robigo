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
pub fn special_fn(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    call_special(cx, act, dst, fun, call)
}

/// A special applied to its call in `rho` (`eval`'s SPECIALSXP case,
/// `R_forceAndCall`'s): a language special's call compiled (with the
/// special inlined) and run; a special that is a function over its call
/// run directly. `R_Visible` as `PRIMPRINT` says.
// R: eval.c:eval (SPECIALSXP)
pub fn apply_special(cx: &mut Cx, call: Root<'_>, fun: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    // SAFETY: a rooted primitive of this isolate.
    let off = unsafe { prims::offset(fun.bind(cx).sexp()) };
    let def = prims::state(cx).defs[off];
    match def.kind {
        Kind::Fn(f) => {
            let flag = def.print();
            // a builtin may change what an argument expression evaluates to or
            // look at an argument: values computed early are dropped (crate::early)
            cx.note_effect();
            cx.set_visible(flag != 1);
            f(cx, call, fun, rho, out)?;
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            Ok(())
        }
        Kind::Special(_) => cx.scope(|cx, s| {
            let code = compile::compile_special(cx, call, off)?;
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
                let mut a = c.as_cons().and_then(|c| c.cdr().as_cons());
                while let Some(cell) = a {
                    vals.push(cell.car());
                    names.push(cell.tag().as_sym());
                    a = cell.cdr().as_cons();
                }
                (vals, names)
            };
            let named = names.iter().any(Option::is_some);
            let args = s.args(cx, &vals, named.then_some(&names[..]));
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

/// The isolate's `force_and_call` hook: `R_forceAndCall(call, 1, rho)`
/// for `lapply`'s and `vapply`'s `FUN(X[[i]], ...)`, under its
/// PROTECT(fun) and the arguments' or the call's: a primitive called as
/// `eval` calls it but with no `check_stack_balance`; a closure applied to
/// the call's promised arguments, the first forced before its body runs.
/// A function that needs code to find (a promise) is `eval` of the call.
// R: eval.c:R_forceAndCall
pub fn force_and_call(cx: &mut Cx, call: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let head = call.bind(cx).as_cons().map(|c| c.car().sexp());
    // (a function found with no code to run: `lapply`'s FUN is a value)
    // SAFETY: an immortal symbol and a rooted environment.
    let fun = head
        .filter(|&h| unsafe { h.typ() } == SYMSXP)
        .and_then(|h| unsafe { cx.fun_lookup_raw(h, rho.bind(cx).sexp()) });
    // SAFETY: a function bound in a live frame.
    match fun.map(|f| unsafe { f.typ() }) {
        Some(t @ (BUILTINSXP | SPECIALSXP | CLOSXP)) => cx.scope(|cx, s| {
            // SAFETY: as above; rooted before anything can collect.
            let fr = s.root(cx, unsafe { Ref::from_sexp(fun.unwrap(), cx) });
            cx.with_pp_excess(2, |cx| match t {
                SPECIALSXP => apply_special(cx, call, fr, rho, out),
                BUILTINSXP => builtin_evallist(cx, call, fr, rho, out, false),
                _ => {
                    let pargs = s.out(cx).root();
                    let o = pargs.reset(cx);
                    crate::dispatch::promise_call_args(cx, call, rho, o)?;
                    // the first argument forced: a promise, else GNU R's
                    // errors
                    let first = pargs.bind(cx).as_cons().map(|c| c.car().sexp());
                    if let Some(p) = first {
                        // SAFETY: a live element of the rooted list (nothing
                        // collected since it was read).
                        if unsafe { p.typ() } == PROMSXP {
                            cx.scope(|cx, s2| {
                                // SAFETY: as above; rooted at once.
                                let pr = s2.root(cx, unsafe { Ref::from_sexp(p, cx) });
                                let o = s2.out(cx);
                                cx.eval(pr, rho, o)
                            })?;
                        } else if p == cx.missing_arg().sexp() {
                            return Err(cx.error_call(Some(call), "argument 1 is empty"));
                        } else {
                            return Err(cx.error("something weird happened"));
                        }
                    }
                    crate::call::apply_closure(cx, call, fr, pargs, rho, None, out)
                }
            })
        }),
        _ => cx.eval(call, rho, out),
    }
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
                        builtin_evallist(cx, call, fr, rho, out, false)?;
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
                        crate::args::Source::Site(crate::ir::NO_SITE),
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
