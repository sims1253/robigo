//! errors.c's C-level tryCatch: `R_tryCatch`, `R_tryCatchError` and
//! `R_withCallingErrorHandler` are `api.c`'s, over the entries here, which
//! do their R parts; `.Internal(C_tryCatchHelper(addr, sw, cond))` is how
//! their R code calls the C body, handler and finally back.

use crate::entry;
use robigo_rtcore::{Args, Cx, Out, RResult, Ref, Ret6, Root, Sexp, View};
use std::ffi::{c_int, c_void};

/// `api.c`'s `robigo_trycatch_t`, errors.c's `tryCatchData_t`: the C
/// functions and data of one `R_tryCatch` or `R_withCallingErrorHandler`,
/// in that call's frame.
#[repr(C)]
struct TryCatchData {
    /// `SEXP (*body)(void *)`
    body: *const c_void,
    bdata: *mut c_void,
    /// `SEXP (*handler)(SEXP, void *)`
    handler: *const c_void,
    hdata: *mut c_void,
    /// `void (*finally)(void *)`
    finally: *const c_void,
    fdata: *mut c_void,
    /// `R_interrupts_suspended` at the call to `R_tryCatch`.
    suspended: c_int,
}

entry! {
    /// `R_tryCatch`'s R part: `R_interrupts_suspended` kept in the data
    /// and set, `evalKeepVis(lang4(trycatch_callback, tcdptr, conds, fin),
    /// R_GlobalEnv)`, the old state put back.
    // R: errors.c:R_tryCatch
    [3] fn robigo_rtapi_try_catch(tcd: *mut c_void, conds: Sexp, fin: c_int) -> Sexp => |cx, s| {
        let suspended = cx.interrupts_suspended();
        // SAFETY: api.c's data in R_tryCatch's frame, for this call.
        unsafe { (*tcd.cast::<TryCatchData>()).suspended = suspended as c_int };
        cx.set_interrupts_suspended(true);
        let o = s.out(cx);
        let addr = o.root();
        cx.build(o, |cx| {
            let nil = cx.nil();
            cx.extptr(tcd as usize, nil, nil)
        })?;
        let o = s.out(cx);
        let val = o.root();
        robigo_rt::conditions::eval_trycatch_callback(cx, addr, conds, fin != 0, o)?;
        cx.set_interrupts_suspended(suspended);
        Ok(val.bind(cx).sexp())
    }

    /// `R_withCallingErrorHandler`'s handler made and pushed; the old
    /// `R_HandlerStack`, which `api.c` puts back after the body.
    // R: errors.c:R_withCallingErrorHandler
    [1] fn robigo_rtapi_push_error_handler(tcd: *mut c_void) -> Sexp => |cx, s| {
        let o = s.out(cx);
        let addr = o.root();
        cx.build(o, |cx| {
            let nil = cx.nil();
            cx.extptr(tcd as usize, nil, nil)
        })?;
        let old = robigo_rt::conditions::push_calling_error_handler(cx, s, addr)?;
        Ok(old.bind(cx).sexp())
    }

    /// `R_HandlerStack = oldstack`.
    [1] fn robigo_rtapi_set_handler_stack(old: Sexp) -> () => |cx, _s| {
        cx.set_handler_stack(old);
        Ok(())
    }
}

/// `.Internal(C_tryCatchHelper(addr, sw, cond))`: 0 calls the body (with
/// interrupts enabled for it if they were at `R_tryCatch`), 1 the handler
/// with `cond`, 2 the finally function. Its arity is not checked (-1), and
/// an argument past the end is R_NilValue, as `CAR`, `CADR` and `CADDR`
/// read a short list.
// R: errors.c:do_tryCatchHelper
fn do_try_catch_helper(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = args.len();
    let eptr = if n > 0 { args.get(cx, 0) } else { cx.nil() };
    let View::ExtPtr(p) = eptr.view() else {
        return Err(cx.error("not an external pointer"));
    };
    // SAFETY: R_tryCatch's data in its frame, which stays while the R code
    // it called runs (the external pointer is that call's).
    let t = unsafe { &*(p.addr() as *const TryCatchData) };
    // asInteger(R_NilValue) is NA_INTEGER
    let sw = if n > 1 { robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))? } else { i32::MIN };
    let word = match sw {
        0 => {
            // SAFETY: C's contract for the body: SEXP (*)(void *).
            let call =
                |cx: &mut Cx| unsafe { cx.call_native6(t.body, [t.bdata as usize, 0, 0, 0, 0, 0], 0, Ret6::Word) };
            if t.suspended != 0 {
                call(cx)?
            } else {
                // enabled for the body, suspended again on the way out
                cx.set_interrupts_suspended(false);
                let v = call(cx)?;
                cx.set_interrupts_suspended(true);
                v
            }
        }
        // (R_tryCatch puts the default handler in when C gives none)
        1 => {
            if t.handler.is_null() {
                out.set(cx, cx.nil());
                return Ok(());
            }
            let cond = (if n > 2 { args.get(cx, 2) } else { cx.nil() }).sexp().as_ptr() as usize;
            // SAFETY: C's contract for the handler: SEXP (*)(SEXP, void *);
            // `cond` is the rooted argument (or R_NilValue).
            unsafe { cx.call_native6(t.handler, [cond, t.hdata as usize, 0, 0, 0, 0], 1, Ret6::Word) }?
        }
        2 => {
            if !t.finally.is_null() {
                // SAFETY: C's contract for finally: void (*)(void *).
                unsafe { cx.call_native6(t.finally, [t.fdata as usize, 0, 0, 0, 0, 0], 0, Ret6::Word) }?;
            }
            out.set(cx, cx.nil());
            return Ok(());
        }
        // "should not happen"
        _ => {
            out.set(cx, cx.nil());
            return Ok(());
        }
    };
    let robigo_rtcore::Native6::Word(w) = word else { unreachable!() };
    // SAFETY: the C function returned a live object of this isolate (a
    // SEXP), rooted here before anything can collect.
    let v = unsafe { Ref::from_sexp(Sexp::from_ptr(w as *mut u8), cx) };
    out.set(cx, v);
    Ok(())
}

pub(crate) fn install(cx: &mut Cx) {
    use robigo_rt::prims::{Inline, Kind, PrimDef};
    robigo_rt::define_builtins(
        cx,
        &[
            // names.c: {"C_tryCatchHelper",do_tryCatchHelper,0,11,-1,{PP_FUNCALL, PREC_FN, 0}}
            PrimDef {
                name: "C_tryCatchHelper",
                kind: Kind::Builtin(do_try_catch_helper, Inline::None),
                eval: 11,
                arity: -1,
            },
        ],
    );
}
