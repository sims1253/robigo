//! Applying a function to a call object (GNU R's `eval` of a LANGSXP once
//! the function is found, and `R_forceAndCall`'s): a special applied to its
//! call, and a call made at run time evaluated without compiling it
//! (complex assignment's getter and setter calls). A closure goes through
//! [`crate::call::apply_closure`] (C1's seam), a builtin through `evalList`
//! ([`crate::call::builtin_evallist`]).

#![allow(unsafe_code)]

use crate::call::{apply_closure, builtin_evallist, call_special, promise_args};
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
        Kind::Builtin(..) => unreachable!("robigo: a builtin applied as a special"),
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
            CLOSXP => {
                let args = promise_args(cx, rho, call)?;
                // SAFETY: fresh, of this isolate.
                let args = s.root(cx, unsafe { Ref::from_sexp(args, cx) });
                apply_closure(cx, call, fr, args, rho, None, out)
            }
            _ => Err(cx.error("attempt to apply non-function")),
        }
    });
    cx.set_eval_depth(depthsave);
    r
}

// ---- R_forceAndCall and lapply (S1) --------------------------------------------

/// `R_forceAndCall(e, n, rho)`: the call `e` evaluated in `rho`, a
/// closure's first `n` arguments forced before it is applied (a builtin
/// or special is called as `eval` calls it).
// R: eval.c:R_forceAndCall
pub fn force_and_call(cx: &mut Cx, e: Root<'_>, n: usize, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let head = s.root(cx, e.bind(cx).as_cons().expect("robigo: a call").car());
        let fun = s.out(cx).root();
        if head.bind(cx).sexptype() == SYMSXP {
            let o = fun.reset(cx);
            cx.find_fun3(head, rho, None, o)?;
        } else {
            let o = fun.reset(cx);
            crate::eval(cx, head, rho, o)?;
        }
        match fun.bind(cx).sexptype() {
            CLOSXP => {
                let pargs = s.out(cx).root();
                let o = pargs.reset(cx);
                crate::dispatch::promise_call_args(cx, e, rho, o)?;
                let items: Vec<Root<'_>> = {
                    let mut v = Vec::new();
                    let mut l = pargs.bind(cx);
                    while let Some(c) = l.as_cons() {
                        v.push(c.car());
                        l = c.cdr();
                    }
                    v.into_iter().take(n).map(|p| s.root(cx, p)).collect()
                };
                for (i, p) in items.iter().enumerate() {
                    let pv = p.bind(cx);
                    if pv.sexptype() == PROMSXP {
                        let o = s.out(cx);
                        crate::eval(cx, *p, rho, o)?;
                    } else if pv.sexp() == cx.missing_arg().sexp() {
                        let msg = format!("argument {} is empty", i + 1);
                        return Err(cx.error_call(Some(e), &msg));
                    } else {
                        return Err(cx.error("something weird happened"));
                    }
                }
                crate::call::apply_closure(cx, e, fun, pargs, rho, None, out)
            }
            // eval's own handling of a primitive's call
            BUILTINSXP | SPECIALSXP => crate::eval(cx, e, rho, out),
            _ => Err(cx.error("attempt to apply non-function")),
        }
    })
}

/// `checkArgIsSymbol(x)`.
fn check_symbol(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    if x.bind(cx).sexptype() != SYMSXP {
        return Err(cx.error("invalid argument"));
    }
    Ok(())
}

/// `.Internal(lapply(X, FUN))`: `FUN(X[[i]], ...)` for each `i`, the
/// index bound in the frame as `i` (one integer vector, updated in place
/// unless `FUN` captured it), each value forced and called with
/// `R_forceAndCall`; the list named by `X`'s names.
// R: apply.c:do_lapply
pub fn do_lapply(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        // checkArity: the inner call's two arguments
        let (x, f) = {
            let c = call.bind(cx).as_cons().unwrap();
            let a = c.cdr();
            let n = {
                let mut k = 0;
                let mut l = a;
                while let Some(cell) = l.as_cons() {
                    k += 1;
                    l = cell.cdr();
                }
                k
            };
            if n != 2 {
                let msg = format!(
                    "{n} argument{} passed to .Internal(lapply) which requires 2",
                    if n == 1 { "" } else { "s" }
                );
                return Err(cx.error_call(Some(call), &msg));
            }
            let a1 = a.as_cons().unwrap();
            (s.root(cx, a1.car()), s.root(cx, a1.cdr().as_cons().unwrap().car()))
        };
        check_symbol(cx, x)?;
        let xx = s.out(cx).root();
        let o = xx.reset(cx);
        crate::eval(cx, x, rho, o)?;
        check_symbol(cx, f)?;
        if xx.bind(cx).is_package_altrep() {
            return Err(cx.error("robigo: lapply() over a package ALTREP vector is not implemented yet"));
        }
        let n = robigo_rtbuiltins::array::r_length(cx, xx.bind(cx));
        if n > i32::MAX as usize {
            return Err(cx.error("robigo: lapply() over a long vector is not implemented yet"));
        }
        let names = robigo_rtbuiltins::attrib::get_value(xx.bind(cx), b"names").map(|nm| s.root(cx, nm));
        // FUN(X[[i]], ...)
        let isym = s.out(cx).root();
        let o = isym.reset(cx);
        cx.build(o, |cx| Ok(cx.install(b"i")?.as_ref()))?;
        let fcall = s.out(cx).root();
        let o = fcall.reset(cx);
        cx.build(o, |cx| {
            let nil = cx.nil();
            let b2 = cx.install(b"[[")?;
            let dots = cx.install(b"...")?;
            let tmp = cx.lcons(b2.as_ref(), cx.cons(x.bind(cx), cx.cons(isym.bind(cx), nil)?)?)?;
            cx.lcons(f.bind(cx), cx.cons(tmp, cx.cons(dots.as_ref(), nil)?)?)
        })?;
        // MARK_NOT_MUTABLE(R_fcall)
        // SAFETY: a fresh call; REFCNT is a header field.
        unsafe { crate::eval::mark_not_mutable(fcall.bind(cx).sexp()) };
        // the index: one integer vector bound as `i`
        let ind = s.out(cx).root();
        let o = ind.reset(cx);
        cx.build(o, |cx| Ok(cx.alloc_copy(&[0i32])?.finish().into()))?;
        cx.define_var(isym, ind, rho)?;
        let results: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
        for (i, slot) in results.iter().enumerate() {
            // the bound index is still ours and unshared: update it in
            // place; else a new one
            let ours = {
                let robigo_rtcore::View::Env(e) = rho.bind(cx).view() else { unreachable!() };
                let sym = isym.bind(cx).as_sym().unwrap();
                match cx.frame_binding(e, sym) {
                    robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
                        robigo_rtcore::BindingValue::Value(v) => v == ind.bind(cx) && !v.maybe_shared(),
                        _ => false,
                    },
                    _ => false,
                }
            };
            if !ours {
                let o = ind.reset(cx);
                cx.build(o, |cx| Ok(cx.alloc_copy(&[0i32])?.finish().into()))?;
                cx.define_var(isym, ind, rho)?;
            }
            // SAFETY: a live INTSXP of one element (rooted), unshared.
            unsafe { *(ind.bind(cx).sexp().std_data() as *mut i32) = i as i32 + 1 };
            let o = slot.reset(cx);
            force_and_call(cx, fcall, 1, rho, o)?;
            // (MAYBE_REFERENCED(tmp): lazy_duplicate is the value itself
            // under reference counting)
        }
        cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in results.iter().enumerate() {
                v.set(i, r.bind(cx));
            }
            if let Some(nm) = names {
                let nsym = cx.install(b"names")?;
                let attrs = cx.cons_tagged(nm.bind(cx), cx.nil(), nsym)?;
                v.set_attrib(attrs);
            }
            Ok(v.finish().into())
        })
    })
}

/// `.Internal(do.call(what, args, envir))`: the call `what(args...)` (the
/// function itself, or the symbol a string names), tagged by the list's
/// non-blank names, evaluated in `envir`.
// R: coerce.c:do_docall
pub fn do_docall(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    if args.len() != 3 {
        let n = args.len();
        let msg =
            format!("{n} argument{} passed to .Internal(do.call) which requires 3", if n == 1 { "" } else { "s" });
        return Err(cx.error_call(Some(call), &msg));
    }
    let (fun, a, envir) = (args.get(cx, 0), args.get(cx, 1), args.get(cx, 2));
    let is_fun = matches!(fun.sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP);
    if !(is_fun || (fun.sexptype() == STRSXP && fun.len() == Some(1))) {
        return Err(cx.error("'what' must be a function or character string"));
    }
    if !matches!(a.sexptype(), NILSXP | VECSXP) {
        return Err(cx.error("'args' must be a list"));
    }
    if envir.sexptype() != ENVSXP {
        return Err(cx.error("'envir' must be an environment"));
    }
    let head: Option<Vec<u8>> = match fun.as_vector::<robigo_rtcore::Char<'_>>() {
        Some(v) => {
            let c = robigo_rtbuiltins::coerce::text(v.get(0));
            let b = robigo_rtbuiltins::text::to_native(c, cx.locale())
                .map(|b| b.into_owned())
                .unwrap_or_else(|_| c.bytes().to_vec());
            if b == b".Internal" {
                return Err(cx.error("illegal usage"));
            }
            Some(b)
        }
        None => {
            if fun.sexptype() == SPECIALSXP && prims::prim_name(cx, fun) == Some(".Internal") {
                return Err(cx.error("illegal usage"));
            }
            None
        }
    };
    let n = robigo_rtbuiltins::array::r_length(cx, a);
    let names: Vec<Option<Vec<u8>>> = {
        let nm =
            robigo_rtbuiltins::attrib::get_value(a, b"names").and_then(|nm| nm.as_vector::<robigo_rtcore::Char<'_>>());
        (0..n).map(|i| nm.map(|v| v.get(i).bytes().to_vec()).filter(|b| !b.is_empty())).collect()
    };
    cx.scope(|cx, s| {
        let c = s.out(cx).root();
        let o = c.reset(cx);
        cx.build(o, |cx| {
            let mut l = cx.nil();
            if n > 0 {
                let v = args.get(cx, 1).as_vector::<Ref<'_>>().unwrap();
                for i in (0..n).rev() {
                    l = match &names[i] {
                        Some(b) => cx.cons_tagged(v.get(i), l, cx.install(b)?)?,
                        None => cx.cons(v.get(i), l)?,
                    };
                }
            }
            let h = match &head {
                Some(b) => cx.install(b)?.as_ref(),
                None => args.get(cx, 0),
            };
            cx.lcons(h, l)
        })?;
        crate::eval(cx, c, args.root(2), out)
    })
}
