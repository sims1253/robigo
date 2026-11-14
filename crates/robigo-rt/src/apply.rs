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
            // SAFETY: the code and environment rooted; the destination is
            // an empty slot.
            unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false) }
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
    let r = cx.scope(|cx, s| {
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
        // SAFETY: a rooted object.
        match unsafe { fr.bind(cx).sexp().typ() } {
            SPECIALSXP => apply_special(cx, call, fr, rho, out),
            BUILTINSXP => builtin_evallist(cx, call, fr, rho, out),
            // the arguments from the call in rho, as promiseArgs makes them;
            // a call made at run time has no call site (no match plan)
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
    });
    cx.set_eval_depth(depthsave);
    r
}
