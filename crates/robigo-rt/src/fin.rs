//! Finalizers: GNU R's `RunFinalizers` and `R_RunWeakRefFinalizer`
//! (memory.c), run where GNU R runs them (docs/g3.md §3 item 7): the
//! evaluation-count polls (`eval.rs`) and after `gc()`.
//!
//! The registration side is `reg.finalizer` (B2b's, over
//! [`register_finalizer`]) and robigo-rtapi's `R_RegisterCFinalizerEx` &
//! co; a C finalizer runs here as a native entry.

#![allow(unsafe_code)]

use robigo_heap::types::*;
use robigo_heap::{Sexp, gp};
use robigo_rtcore::{Cx, RResult, Ref, Root};

/// `R_RegisterFinalizerEx(s, fun, onexit)` for an R function `fun`:
/// GNU R's "can only weakly reference/finalize reference objects" for a
/// key that is not an environment or external pointer.
// R: memory.c:R_RegisterFinalizerEx, NewWeakRef
pub fn register_finalizer(cx: &mut Cx, key: Root<'_>, fun: Root<'_>, onexit: bool) -> RResult<()> {
    let k = key.bind(cx).sexp();
    // SAFETY: rooted.
    if !matches!(unsafe { k.typ() }, NILSXP | ENVSXP | EXTPTRSXP | BCODESXP) {
        return Err(cx.error("can only weakly reference/finalize reference objects"));
    }
    let mut retried = None;
    loop {
        let (k, f, nil) = (key.bind(cx).sexp(), fun.bind(cx).sexp(), cx.nil().sexp());
        // SAFETY: rooted objects of this isolate; the reference is rooted by
        // the collector's list once made.
        match unsafe { cx.new_weak_ref_raw(k, nil, f, onexit) } {
            Ok(_) => return Ok(()),
            Err(e) if retried == Some(e) => {
                return cx.retry_alloc(|_| Err::<(), _>(e));
            }
            Err(e) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

/// `R_RunPendingFinalizers()`: `RunFinalizers()` if any are pending. A
/// finalizer runs in a top-level context of its own, so an error in it
/// does not reach the code that polled.
// R: memory.c:R_RunPendingFinalizers, RunFinalizers
pub fn run_pending_finalizers(cx: &mut Cx) {
    if !cx.finalizers_pending() {
        return;
    }
    let st = crate::prims::state(cx);
    if st.running_finalizers.get() {
        return;
    }
    st.running_finalizers.set(true);
    while let Some(w) = cx.pop_finalizable_raw() {
        cx.scope(|cx, s| {
            // SAFETY: just popped: live, and rooted from here on.
            let w = s.root(cx, unsafe { Ref::from_sexp(w, cx) });
            // R: "the entry in the weak reference list is removed before
            // running the finalizer" (the heap's queue did that)
            let (hs, rs) = (s.root(cx, cx.handler_stack()), s.root(cx, cx.restart_stack()));
            let vis = cx.visible();
            let nil = s.root(cx, cx.nil());
            cx.set_handler_stack(nil);
            cx.set_restart_stack(nil);
            let global = s.root(cx, cx.global_env());
            let base = s.root(cx, cx.base_env());
            let _ = cx.toplevel(global, base, |cx| run_weak_ref_finalizer(cx, w));
            cx.set_handler_stack(hs);
            cx.set_restart_stack(rs);
            cx.set_visible(vis);
        });
    }
    crate::prims::state(cx).running_finalizers.set(false);
}

/// `R_RunWeakRefFinalizer(w)`: clear the reference, then call the
/// finalizer on the key in the global environment.
// R: memory.c:R_RunWeakRefFinalizer
fn run_weak_ref_finalizer(cx: &mut Cx, w: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let ws = w.bind(cx).sexp();
        let nil = cx.nil().sexp();
        // SAFETY: a rooted WEAKREFSXP (key, value, finalizer, next).
        let (key, fun) = unsafe {
            let d = ws.std_data() as *const Sexp;
            (*d, *d.add(2))
        };
        // SAFETY: as above.
        let key = s.root(cx, unsafe { Ref::from_sexp(key, cx) });
        // SAFETY: as above.
        let fun = s.root(cx, unsafe { Ref::from_sexp(fun, cx) });
        // SAFETY: as above: SET_WEAKREF_KEY/VALUE/FINALIZER(w, R_NilValue),
        // SET_READY_TO_FINALIZE(w).
        unsafe {
            ws.set_elt(0, nil);
            ws.set_elt(1, nil);
            ws.set_elt(2, nil);
            ws.set_gp(ws.gp() | gp::READY_TO_FINALIZE);
        }
        let susp = cx.interrupts_suspended();
        cx.set_interrupts_suspended(true);
        let f = fun.bind(cx);
        let r = if let Some(cfun) = c_finalizer(f) {
            // a C finalizer (MakeCFinalizer's raw vector): cfun(key), as a
            // native entry whose argument C now holds
            let k = key.bind(cx).sexp().0 as usize;
            // SAFETY: the raw vector holds the `void (*)(SEXP)`
            // R_RegisterCFinalizerEx registered; the key is rooted above.
            unsafe { cx.call_native6(cfun, [k, 0, 0, 0, 0, 0], 1, robigo_rtcore::Ret6::Word) }.map(|_| ())
        } else if f.sexp() != nil {
            // e = LCONS(fun, LCONS(key, R_NilValue)); eval(e, R_GlobalEnv)
            let e = cx.retry_alloc(|cx| {
                let nil = cx.nil();
                let a = cx.lcons(key.bind(cx), nil)?;
                Ok(cx.lcons(fun.bind(cx), a)?.sexp())
            });
            match e {
                Ok(e) => {
                    // SAFETY: fresh.
                    let e = s.root(cx, unsafe { Ref::from_sexp(e, cx) });
                    let g = s.root(cx, cx.global_env());
                    let o = s.out(cx);
                    crate::eval::eval(cx, e, g, o)
                }
                Err(e) => Err(e),
            }
        } else {
            Ok(())
        };
        cx.set_interrupts_suspended(susp);
        r
    })
}

/// GNU R's `isCFinalizer(fun)` and `GetCFinalizer(fun)`: a C finalizer is
/// kept as a raw vector holding its address (`MakeCFinalizer`).
// R: memory.c:isCFinalizer, GetCFinalizer
fn c_finalizer(fun: Ref<'_>) -> Option<*const std::ffi::c_void> {
    let b = fun.as_vector::<u8>()?.as_slice()?;
    let w: [u8; size_of::<usize>()] = b.try_into().ok()?;
    Some(usize::from_ne_bytes(w) as *const std::ffi::c_void)
}

/// `gc()`'s collection and its `R_RunPendingFinalizers` (memory.c:3093):
/// what the R-level `gc()` builtin (B2b) does around its report.
pub fn gc(cx: &mut Cx) {
    cx.collect();
    run_pending_finalizers(cx);
}
