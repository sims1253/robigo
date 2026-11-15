//! Package code through native entries (docs/g3.md §6.5): E8's cases 2
//! and 3 (an `R_UnwindProtect` cleanup that catches an error resumes the
//! original continuation; a C context's `cend` runs before the jump), with
//! C stand-ins for package code in `src/test_routines.c` that reach the API
//! through `Hooks`, the tests' stand-ins for robigo-rtapi's exports, which
//! jump as export wrappers do. Not under Miri (C code).

#![cfg(not(miri))]
#![allow(clippy::undocumented_unsafe_blocks)]

mod common;
use common::*;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Context, ContextId, Cx, FunctionContext, Native6, Promargs, RResult, Ref, Ret6, Sexp, ctxt, enter_raw,
};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

// ---- package code: native entries (E8 cases 2 and 3) ---------------------------

/// The hooks the C stand-ins reach the API through (`test_routines.c`'s
/// `rt_hooks`).
#[repr(C)]
struct Hooks {
    error: extern "C" fn(*const c_char),
    log: extern "C" fn(*const c_char),
    begin: extern "C" fn(*mut Context, c_int),
    end: extern "C" fn(*mut Context),
    on_chain: extern "C" fn(*mut Context) -> c_int,
    toplevel_exec: extern "C" fn(unsafe extern "C" fn(*mut c_void), *mut c_void) -> c_int,
    unwind_protect:
        extern "C" fn(*const c_void, *mut c_void, unsafe extern "C" fn(*mut c_void, c_int), *mut c_void, Sexp) -> Sexp,
    continue_unwind: extern "C" fn(Sexp),
    error_here: extern "C" fn(*const c_char),
    look: extern "C" fn(),
    invisible: extern "C" fn(),
}

static HOOKS: Hooks = Hooks {
    error: h_error,
    log: h_log,
    begin: h_begin,
    end: h_end,
    on_chain: h_on_chain,
    toplevel_exec: h_toplevel_exec,
    unwind_protect: h_unwind_protect,
    continue_unwind: h_continue_unwind,
    error_here: h_error_here,
    look: h_look,
    invisible: h_invisible,
};

unsafe extern "C" {
    fn rt_test_cend_case(h: *const Hooks) -> Sexp;
    fn rt_test_cend_raises(h: *const Hooks) -> Sexp;
    fn rt_test_cend_invisible(h: *const Hooks) -> Sexp;
    fn rt_test_unwind_case(h: *const Hooks, cont: Sexp) -> Sexp;
    fn rt_test_cpp11_case(h: *const Hooks, cont: Sexp) -> Sexp;
    fn rt_test_unwind_returns(h: *const Hooks, cont: Sexp) -> Sexp;
    fn rt_test_error_here(h: *const Hooks) -> Sexp;
    fn rt_test_look(h: *const Hooks) -> Sexp;
    fn rt_test_exec_fun(d: *mut c_void) -> Sexp;
    fn rt_test_exec_cleanup(d: *mut c_void);
    fn rt_test_toplevel_fun(d: *mut c_void);
    // (addresses only: `double f(long, long)` and `Rcomplex f(long, long)`,
    // called through `call_native6`)
    fn rt_test_half();
    fn rt_test_cplx();
}

/// `test_routines.c`'s `exec_data`.
#[repr(C)]
struct ExecData {
    h: *const Hooks,
    value: Sexp,
    fail: c_int,
    cleanups: c_int,
}

/// An export wrapper: run `f` with a capability; an exit leaves by
/// `longjmp` from here, once the capability has ended.
fn export<R>(f: impl FnOnce(&mut Cx) -> RResult<R>) -> R {
    let r = unsafe { enter_raw(ISO.get(), |cx| f(cx).map_err(|e| cx.prepare_jump(e))) };
    match r {
        Ok(v) => v,
        Err(l) => unsafe { l.jump() },
    }
}

fn cstr(p: *const c_char) -> String {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned()
}

// (The hooks that jump own nothing in their own frames: what needs a
// destructor lives in the export's callback, which has returned when the
// export jumps.)
extern "C" fn h_error(msg: *const c_char) {
    export(|cx| {
        let msg = cstr(msg);
        cx.scope(|cx, s| {
            let m = s.root(cx, string(cx, &msg));
            Err::<(), _>(stop(cx, m))
        })
    })
}

extern "C" fn h_log(msg: *const c_char) {
    log(cstr(msg));
}

extern "C" fn h_begin(c: *mut Context, flag: c_int) {
    export(|cx| {
        let nil = cx.nil().sexp();
        unsafe { cx.begin_foreign(NonNull::new(c).unwrap(), flag, nil, nil, nil, nil, nil) };
        Ok(())
    })
}

extern "C" fn h_end(c: *mut Context) {
    export(|cx| {
        unsafe { cx.end_foreign(NonNull::new(c).unwrap()) };
        Ok(())
    })
}

extern "C" fn h_on_chain(c: *mut Context) -> c_int {
    export(|cx| Ok(cx.context(ContextId::from_ptr(c).unwrap()).is_some() as c_int))
}

extern "C" fn h_toplevel_exec(fun: unsafe extern "C" fn(*mut c_void), data: *mut c_void) -> c_int {
    export(|cx| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            unsafe { cx.toplevel_exec(nil, nil, fun, data) }.map(|ok| ok as c_int)
        })
    })
}

/// `R_UnwindProtect`'s export: the protected call in a callback, the
/// cleanup from this frame (C may leave it by `longjmp`), then the
/// unwind resumed.
extern "C" fn h_unwind_protect(
    fun: *const c_void,
    data: *mut c_void,
    cleanfun: unsafe extern "C" fn(*mut c_void, c_int),
    cleandata: *mut c_void,
    cont: Sexp,
) -> Sexp {
    let jumped = export(|cx| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let cont = s.root(cx, unsafe { Ref::from_sexp(cont, cx) });
            unsafe { cx.unwind_protect(nil, nil, fun, data, cont) }
        })
    });
    unsafe { cleanfun(cleandata, jumped as c_int) };
    if jumped {
        h_continue_unwind(cont);
    }
    unsafe { cont.car() }
}

extern "C" fn h_continue_unwind(cont: Sexp) {
    export(|cx| {
        cx.scope(|cx, s| {
            let cont = s.root(cx, unsafe { Ref::from_sexp(cont, cx) });
            Err::<(), _>(cx.continue_unwind(cont))
        })
    })
}

extern "C" fn h_error_here(msg: *const c_char) {
    export(|cx| {
        let msg = cstr(msg);
        Err::<(), _>(cx.error(&msg))
    })
}

extern "C" fn h_invisible() {
    export(|cx| {
        cx.set_visible(false);
        Ok(())
    })
}

extern "C" fn h_look() {
    export(|cx| {
        let g = cx.global_context();
        let call = g.call();
        let what = if call.sexptype() == STRSXP { text(call) } else { format!("type {}", call.sexptype()) };
        LOOKED.with_borrow_mut(|l| *l = Some((g.callflag(), what)));
        Ok(())
    })
}

/// `.Call(f, args...)` from a function record whose call is the string
/// `call`, inside a `(catch ...)`: the routine's result or the message that
/// reached the catcher.
fn dot_call(cx: &mut Cx, call: &str, f: *const c_void, args: &[*mut c_void]) -> Result<(), String> {
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let fcall = s.root(cx, string(cx, call));
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call: fcall,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let out = s.out(cx);
        let res = out.root();
        let r = cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
            CATCHERS.with_borrow_mut(|c| c.push(id));
            let r = cx.scope(|cx, s| {
                let dcall = s.root(cx, string(cx, ".Call"));
                let mut a = [std::ptr::null_mut::<c_void>(); 65];
                a[..args.len()].copy_from_slice(args);
                unsafe { cx.call_native65(dcall, nil, f, &mut a) }.map(|_| ())
            });
            CATCHERS.with_borrow_mut(|c| c.pop());
            r?;
            out.set(cx, string(cx, "returned"));
            Ok(())
        });
        match r {
            Ok(()) => {
                let t = text(res.bind(cx));
                if t == "returned" { Ok(()) } else { Err(t) }
            }
            Err(e) => {
                let m = text(cx.exit_objects(&e).0);
                cx.resolve_exit(e, None);
                panic!("an exit passed the catcher: {m}")
            }
        }
    })
}

fn hooks() -> *mut c_void {
    (&raw const HOOKS).cast_mut().cast()
}

/// E8: a C context's `cend` runs before the jump, while the frame that
/// holds the record is still there and the record is on the chain; then
/// the on.exit code of the closure the jump crosses, on its way to the
/// catcher outside it.
#[test]
fn e8_a_c_contexts_cend_runs_before_the_jump() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let fenv = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let code = s.root(cx, l!(cx, "log", string(cx, "on.exit")));
        let catcher = FunctionContext {
            callflag: ctxt::RETURN,
            call: nil,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let f = FunctionContext { cloenv: fenv, ..catcher };
        let out = s.out(cx);
        let res = out.root();
        cx.function_context(&catcher, out, &mut eval_discard, |cx, id, out| {
            CATCHERS.with_borrow_mut(|c| c.push(id));
            let r = cx.function_context(&f, out, &mut eval_discard, |cx, _, _| {
                cx.on_exit(nil, fenv, code, None, None)?;
                let mut a = [std::ptr::null_mut::<c_void>(); 65];
                a[0] = hooks();
                unsafe { cx.call_native65(nil, nil, rt_test_cend_case as *const c_void, &mut a) }?;
                log("returned");
                Ok(())
            });
            CATCHERS.with_borrow_mut(|c| c.pop());
            r
        })
        .unwrap();
        text(res.bind(cx))
    });
    assert_eq!(r, "from C");
    assert_eq!(take_log(), ["cend: before the jump", "on.exit"]);
    assert_eq!(cx.context_depth(), 1);
}

/// A `cend` that raises an error while another error unwinds: the new
/// error supersedes it, and the outer C record's `cend` still runs, once.
#[test]
fn a_cend_that_raises_supersedes_the_jump() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = dot_call(cx, "f()", rt_test_cend_raises as *const c_void, &[hooks()]);
    assert_eq!(r, Err("from the cend".into()));
    assert_eq!(take_log(), ["inner cend", "outer cend"]);
    assert_eq!(cx.pending_exits(), 0);
    assert_eq!(cx.context_depth(), 1);
}

/// The superseding exit's leg decides `R_Visible`: a `cend` that makes
/// the result invisible and then raises leaves it invisible (GNU R never
/// returns to the `R_jumpctxt` that would restore the first exit's).
#[test]
fn a_superseding_cends_visibility_stands() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_visible(true);
    let r = dot_call(cx, "f()", rt_test_cend_invisible as *const c_void, &[hooks()]);
    assert_eq!(r, Err("from the cend".into()));
    assert!(!cx.visible());
}

/// E8: an `R_UnwindProtect` cleanup that catches an error of its own
/// (`R_ToplevelExec`) resumes the original continuation: the catcher gets
/// "outer".
#[test]
fn e8_an_unwind_protect_cleanup_that_catches_resumes_the_original_continuation() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = cx.scope(|cx, s| {
        let cont = cx.make_unwind_cont().unwrap();
        let cont = s.root(cx, cont);
        let c = cont.bind(cx).sexp().0.cast::<c_void>();
        dot_call(cx, "f()", rt_test_unwind_case as *const c_void, &[hooks(), c])
    });
    assert_eq!(r, Err("outer".into()));
    assert_eq!(take_log(), ["fun", "cleanup: jump", "cleanup: inner caught"]);
    assert_eq!(cx.pending_exits(), 0);
    assert_eq!(cx.context_depth(), 1);
}

/// cpp11's shape: the cleanup leaves by `longjmp` to the routine, which
/// resumes the unwind later with `R_ContinueUnwind`: the continuation owns
/// the exit's record in between.
#[test]
fn a_continuation_owns_its_exit_until_it_is_resumed() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = cx.scope(|cx, s| {
        let cont = s.root(cx, cx.make_unwind_cont().unwrap());
        let c = cont.bind(cx).sexp().0.cast::<c_void>();
        dot_call(cx, "f()", rt_test_cpp11_case as *const c_void, &[hooks(), c])
    });
    assert_eq!(r, Err("outer".into()));
    assert_eq!(take_log(), ["fun", "cleanup: jump", "routine: unwinding C++ frames"]);
    assert_eq!(cx.pending_exits(), 0);
}

#[test]
fn unwind_protect_without_a_jump_returns_funs_value() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = cx.scope(|cx, s| {
        let cont = s.root(cx, cx.make_unwind_cont().unwrap());
        let c = cont.bind(cx).sexp().0.cast::<c_void>();
        dot_call(cx, "f()", rt_test_unwind_returns as *const c_void, &[hooks(), c])
    });
    assert_eq!(r, Ok(()));
    assert_eq!(take_log(), ["cleanup: no jump", "cleanup: inner caught", "after R_UnwindProtect"]);
}

/// `R_ToplevelExec`: an error inside lands at its record (FALSE), and
/// `R_ToplevelContext` and the handler stack come back.
#[test]
fn toplevel_exec_lands_errors_and_restores() {
    let mut iso = iso();
    let cx = iso.cx();
    let top = cx.toplevel_context().id();
    for fail in [0, 1] {
        let mut d = ExecData { h: &HOOKS, value: Sexp::NULL, fail, cleanups: 0 };
        let ok = cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            unsafe { cx.toplevel_exec(nil, nil, rt_test_toplevel_fun, (&raw mut d).cast()) }
        });
        assert_eq!(ok.unwrap(), fail == 0);
        assert_eq!(cx.toplevel_context().id(), top);
        assert_eq!(cx.pending_exits(), 0);
    }
    assert_eq!(take_log(), ["toplevel fun", "toplevel fun"]);
}

/// `R_ExecWithCleanup`: the cleanup runs once on a normal return, once
/// (in the jump's walk) when `fun` fails, and twice when the cleanup
/// itself fails (it is the record's `cend` too), as in GNU R.
#[test]
fn exec_with_cleanup_runs_its_cleanup_as_gnu_r_does() {
    let mut iso = iso();
    let cx = iso.cx();
    for (fail, cleanups, ok) in [(0, 1, true), (1, 1, false), (2, 2, false)] {
        let r = cx.scope(|cx, s| {
            let rho = s.root(cx, env(cx));
            let nil = s.root(cx, cx.nil());
            let value = s.root(cx, string(cx, "fun's value"));
            let spec = FunctionContext {
                callflag: ctxt::RETURN,
                call: nil,
                cloenv: rho,
                sysparent: rho,
                callfun: nil,
                promargs: Promargs::Filled(nil),
            };
            let out = s.out(cx);
            let res = out.root();
            let mut d = ExecData { h: &HOOKS, value: value.bind(cx).sexp(), fail, cleanups: 0 };
            let r = cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
                CATCHERS.with_borrow_mut(|c| c.push(id));
                let r = unsafe {
                    cx.exec_with_cleanup(
                        nil,
                        rt_test_exec_fun as *const c_void,
                        (&raw mut d).cast(),
                        rt_test_exec_cleanup,
                        (&raw mut d).cast(),
                        out,
                    )
                };
                CATCHERS.with_borrow_mut(|c| c.pop());
                r
            });
            assert!(r.is_ok());
            assert_eq!(d.cleanups, cleanups);
            text(res.bind(cx))
        });
        assert_eq!(r == "fun's value", ok, "{r}");
        take_log();
    }
}

/// `error()` from a routine reports the closure's call: the `.Call`'s
/// `CTXT_BUILTIN` record is skipped, as GNU R's `Rf_error` skips it.
#[test]
fn error_reports_the_call_below_a_foreign_record() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let call = s.root(cx, string(cx, "f(x)"));
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let out = s.out(cx);
        let r = cx.function_context(&spec, out, &mut eval_discard, |cx, _, _| {
            cx.scope(|cx, s| {
                let dcall = s.root(cx, string(cx, ".Call(...)"));
                let mut a = [std::ptr::null_mut::<c_void>(); 65];
                a[0] = hooks();
                unsafe { cx.call_native65(dcall, nil, rt_test_error_here as *const c_void, &mut a) }.map(|_| ())
            })
        });
        let e = r.unwrap_err();
        let (msg, _, call) = cx.exit_objects(&e);
        assert_eq!(text(msg), "from error()");
        assert_eq!(text(call), "f(x)");
        cx.resolve_exit(e, None);
    });
}

/// A callback GNU R makes no context for (an ALTREP method) runs from a
/// hidden record: R code sees the closure's record as the current one.
#[test]
fn hidden_records_are_invisible_to_r_code() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let call = s.root(cx, string(cx, "g()"));
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let out = s.out(cx);
        cx.function_context(&spec, out, &mut eval_discard, |cx, _, out| {
            let depth = cx.context_depth();
            unsafe {
                cx.call_native6(rt_test_look as *const c_void, [hooks() as usize, 0, 0, 0, 0, 0], 0, Ret6::Word)
            }?;
            assert_eq!(cx.context_depth(), depth);
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
    assert_eq!(LOOKED.take(), Some((ctxt::RETURN, "g()".to_string())));
}

#[test]
fn six_argument_entries_return_words_doubles_and_complexes() {
    let mut iso = iso();
    let cx = iso.cx();
    let a = [40usize, 3, 0, 0, 0, 0];
    assert_eq!(
        unsafe { cx.call_native6(rt_test_half as *const c_void, a, 0, Ret6::Double) }.unwrap(),
        Native6::Double(21.5)
    );
    assert_eq!(
        unsafe { cx.call_native6(rt_test_cplx as *const c_void, a, 0, Ret6::Complex) }.unwrap(),
        Native6::Complex([40.0, 3.0])
    );
}
