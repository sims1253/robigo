//! apply.c's `lapply` (a special `.Internal` that calls back into R), and
//! eval.c's `R_forceAndCall`, which it calls.

#![allow(unsafe_code)]

use robigo_heap::types::*;
use robigo_rtcore::{Cx, Out, RResult, Ref, Root};

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
                let msg = format!("{n} argument{} passed to .Internal(lapply) which requires 2", if n == 1 { "" } else { "s" });
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
