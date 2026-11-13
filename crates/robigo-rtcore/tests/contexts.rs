//! Contexts and unwinding (docs/g3.md §6.5), tested the way the evaluator,
//! the C API exports and package code use them: E8's three cases (an
//! `on.exit` that catches its own error during an unwind returns the outer
//! condition; an `R_UnwindProtect` cleanup that catches an error resumes
//! the original continuation; a C context's `cend` runs before the jump),
//! GNU R's `on.exit`/`return`/superseding rules, the native entries, the
//! frame walks, the limits, and the chain as a precise root.
//!
//! R code is a small test language evaluated over the real API: `(fn
//! BODY)` calls a closure (a function record with a fresh frame), `(catch
//! EXPR)` an exiting handler (a function record that `(stop MSG)` targets,
//! as S1's `tryCatch` does), `(onexit CODE [ADD [AFTER]])` is `on.exit`,
//! `(return VAL)`, `(log MSG)`, `(seq ...)`, `(gc)` a collection, and a
//! string is itself. C stand-ins for package code are in
//! `src/test_routines.c`; they reach the API through `Hooks`, the tests'
//! stand-ins for robigo-rtapi's exports, which jump as export wrappers do.

#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::types::*;
use robigo_rtcore::{
    Char, Context, ContextId, Cx, Encoding, Exit, ExitInfo, ExitKind, Frame, FunctionContext, Isolate, Left, Native6,
    Options, Out, OwnedIsolate, Promargs, RError, RResult, Ref, Ret6, Root, Sexp, WalkError, ctxt, enter_raw,
};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

thread_local! {
    static ISO: Cell<*mut Isolate> = const { Cell::new(std::ptr::null_mut()) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// the innermost `(catch ...)` records, innermost last
    static CATCHERS: RefCell<Vec<ContextId>> = const { RefCell::new(Vec::new()) };
    static LOOKED: RefCell<Option<(i32, String)>> = const { RefCell::new(None) };
    static FILLS: Cell<u32> = const { Cell::new(0) };
}

fn log(s: impl Into<String>) {
    LOG.with_borrow_mut(|l| l.push(s.into()));
}

fn take_log() -> Vec<String> {
    LOG.with_borrow_mut(std::mem::take)
}

fn iso() -> OwnedIsolate {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() });
    let p = iso.cx().call_out(|re| re.as_ptr());
    ISO.set(p);
    take_log();
    CATCHERS.with_borrow_mut(|c| c.clear());
    iso
}

// ---- values -------------------------------------------------------------

fn string<'g>(cx: &'g Cx, s: &str) -> Ref<'g> {
    let c = cx.mk_char(s.as_bytes(), Encoding::Utf8).unwrap();
    let mut v = cx.alloc_vector::<Char<'_>>(1).unwrap();
    v.set(0, c);
    v.finish().into()
}

fn text(r: Ref<'_>) -> String {
    match r.as_vector::<Char<'_>>() {
        Some(v) => String::from_utf8(v.get(0).bytes().to_vec()).unwrap(),
        None if r.sexptype() == NILSXP => "NULL".into(),
        None => panic!("not a string: type {}", r.sexptype()),
    }
}

/// A call `(head args...)`.
fn lang<'g>(cx: &'g Cx, head: &str, args: &[Ref<'g>]) -> Ref<'g> {
    let mut l = cx.nil();
    for &a in args.iter().rev() {
        l = cx.cons(a, l).unwrap();
    }
    cx.lcons(cx.install(head.as_bytes()).unwrap().into(), l).unwrap()
}

/// A fresh object standing for an environment (records compare frames by
/// identity only).
fn env<'g>(cx: &'g Cx) -> Ref<'g> {
    cx.cons(cx.nil(), cx.nil()).unwrap()
}

// ---- the test language ----------------------------------------------------

fn args_of<'g>(e: Ref<'g>) -> (String, Vec<Ref<'g>>) {
    let c = e.as_cons().unwrap();
    let head = String::from_utf8(c.car().as_sym().unwrap().name().bytes().to_vec()).unwrap();
    let mut args = Vec::new();
    let mut a = c.cdr();
    while let Some(cell) = a.as_cons() {
        args.push(cell.car());
        a = cell.cdr();
    }
    (head, args)
}

fn eval_discard(cx: &mut Cx, e: Root<'_>, rho: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| eval(cx, e, rho, s.out(cx)))
}

/// An exit to the innermost `(catch ...)` (or the top level), carrying
/// `msg`: what S1's `stop` does once handlers chose the target.
fn stop(cx: &mut Cx, msg: Root<'_>) -> RError {
    // only a catcher inside the current top-level record is seen
    // (`R_ToplevelExec` empties the handler stack)
    let target = CATCHERS.with_borrow(|c| {
        let c = c.last().copied()?;
        let top = cx.toplevel_context().id();
        let mut r = Some(cx.global_context());
        while let Some(x) = r {
            if x.id() == c {
                return Some(c.addr());
            }
            if x.id() == top {
                return None;
            }
            r = x.next();
        }
        None
    });
    let target = target.unwrap_or(0);
    cx.exit(Exit {
        info: ExitInfo { kind: ExitKind::Error, mask: ctxt::FUNCTION, target },
        value: Some(msg),
        condition: None,
        call: None,
    })
}

fn eval(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let x = e.bind(cx);
    if x.sexptype() != LANGSXP {
        out.set(cx, x);
        return Ok(());
    }
    let (head, n) = {
        let (h, a) = args_of(x);
        (h, a.len())
    };
    cx.scope(|cx, s| {
        let a: Vec<Root<'_>> = (0..n).map(|i| s.root(cx, args_of(e.bind(cx)).1[i])).collect();
        match head.as_str() {
            "log" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                log(text(vr.bind(cx)));
                out.set(cx, cx.nil());
                Ok(())
            }
            "stop" => Err(stop(cx, a[0])),
            "return" => {
                let v = s.out(cx);
                let vr = v.root();
                eval(cx, a[0], rho, v)?;
                Err(cx.find_context(ctxt::FUNCTION, rho, Some(vr)))
            }
            "seq" => {
                for &x in &a[..n - 1] {
                    eval_discard(cx, x, rho)?;
                }
                eval(cx, a[n - 1], rho, out)
            }
            "onexit" => {
                let flag = |cx: &Cx, i: usize| (n > i).then(|| (text(a[i].bind(cx)) == "T") as i32);
                let (add, after) = (flag(cx, 1), flag(cx, 2));
                cx.on_exit(e, rho, a[0], add, after)?;
                out.set(cx, cx.nil());
                Ok(())
            }
            "fn" => call_fn(cx, e, a[0], rho, out),
            "catch" => catch(cx, e, a[0], rho, out),
            "gc" => {
                collect(cx);
                out.set(cx, cx.nil());
                Ok(())
            }
            // the value the record whose on.exit code runs returns
            "peek" => {
                let v = cx.exit_context().and_then(|c| c.return_value()).map_or("undefined".into(), text);
                log(format!("returnValue {v}"));
                out.set(cx, cx.nil());
                Ok(())
            }
            h => panic!("unknown test function {h}"),
        }
    })
}

/// `(fn BODY)`: a closure call, its frame fresh, `BODY` evaluated in it.
fn call_fn(cx: &mut Cx, call: Root<'_>, body: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let fenv = s.root(cx, env(cx));
        let fun = s.root(cx, cx.install(b"fn").unwrap());
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: fenv,
            sysparent: rho,
            callfun: fun,
            promargs: Promargs::Filled(nil),
        };
        cx.function_context(&spec, out, &mut eval_discard, |cx, _, out| eval(cx, body, fenv, out))
    })
}

/// `(catch EXPR)`: `EXPR`'s value, or the message of a `(stop ...)` in it.
fn catch(cx: &mut Cx, call: Root<'_>, expr: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let fenv = s.root(cx, env(cx));
        let fun = s.root(cx, cx.install(b"catch").unwrap());
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: fenv,
            sysparent: rho,
            callfun: fun,
            promargs: Promargs::Filled(nil),
        };
        cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
            CATCHERS.with_borrow_mut(|c| c.push(id));
            let r = eval(cx, expr, rho, out);
            CATCHERS.with_borrow_mut(|c| c.pop());
            r
        })
    })
}

/// Run `prog` at the top level: its value as text, or the error that
/// reached the top level.
fn run(cx: &mut Cx, prog: impl for<'g> FnOnce(&'g Cx) -> Ref<'g>) -> Result<String, String> {
    cx.scope(|cx, s| {
        let e = s.root(cx, prog(cx));
        let rho = s.root(cx, env(cx));
        let out = s.out(cx);
        let r = out.root();
        match eval(cx, e, rho, out) {
            Ok(()) => Ok(text(r.bind(cx))),
            Err(err) => {
                let msg = text(cx.exit_objects(&err).0);
                cx.resolve_exit(err, None);
                Err(msg)
            }
        }
    })
}

/// A collection with the isolate's roots: churn until the vector limit
/// refuses, then `need_gc` (the tests' isolates have no safepoints yet).
fn collect(cx: &mut Cx) {
    let refused = (0..10_000).find_map(|_| cx.alloc_vector::<f64>(1 << 15).err()).expect("no allocation was refused");
    cx.need_gc(refused).unwrap();
}

macro_rules! l {
    ($cx:expr, $head:literal $(, $a:expr)*) => { lang($cx, $head, &[$($a),*]) };
}

// ---- the chain --------------------------------------------------------------

#[test]
fn rcntxt_is_laid_out_as_defn_h_declares() {
    robigo_rtcore::check_layout();
    let c = robigo_rtcore::c_layout();
    assert_eq!(c[0], size_of::<Context>());
    assert_eq!(c[1], align_of::<Context>());
    // GNU R's 432 bytes on x86-64, then robigo's
    if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        assert_eq!(size_of::<Context>(), 432 + 72);
    }
}

#[test]
fn the_chain_starts_at_the_session_record() {
    let mut iso = iso();
    let cx = iso.cx();
    assert_eq!(cx.context_depth(), 1);
    let g = cx.global_context().id();
    assert_eq!(g, cx.toplevel_context().id());
    assert_eq!(g, cx.session_context().id());
    assert_eq!(cx.global_context().callflag(), ctxt::TOPLEVEL);
    assert!(cx.exit_context().is_none());
}

#[test]
fn function_records_nest_and_pop() {
    let mut iso = iso();
    let cx = iso.cx();
    let ids: RefCell<Vec<ContextId>> = RefCell::new(Vec::new());
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let fun = s.root(cx, cx.install(b"f").unwrap());
        let spec = FunctionContext { callflag: ctxt::RETURN, call: nil, cloenv: rho, sysparent: rho, callfun: fun, promargs: Promargs::Filled(nil) };
        let out = s.out(cx);
        cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
            ids.borrow_mut().push(id);
            assert_eq!(cx.context_depth(), 2);
            assert_eq!(cx.global_context().id(), id);
            cx.scope(|cx, s| {
                let inner = s.out(cx);
                cx.function_context(&spec, inner, &mut eval_discard, |cx, id2, out| {
                    ids.borrow_mut().push(id2);
                    assert_eq!(cx.context_depth(), 3);
                    assert_eq!(cx.global_context().next().unwrap().id(), id);
                    out.set(cx, cx.nil());
                    Ok(())
                })
            })?;
            assert_eq!(cx.context_depth(), 2);
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
    assert_eq!(cx.context_depth(), 1);
    assert!(ids.borrow().iter().all(|&id| cx.context(id).is_none()), "popped records are off the chain");
}

#[test]
fn a_panic_unwinds_the_chain() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = run(cx, |cx| l!(cx, "fn", l!(cx, "fn", lang(cx, "unknown", &[]))));
    }));
    assert!(r.is_err());
    assert_eq!(cx.context_depth(), 1);
    assert_eq!(cx.stack_depth(), 0);
}

// ---- on.exit, return, superseding (E8 case 1) -------------------------------

#[test]
fn on_exit_runs_after_the_body_and_sees_its_value() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let code = l!(cx, "seq", l!(cx, "log", string(cx, "exit")), l!(cx, "peek"));
        l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", code), string(cx, "value")))
    });
    assert_eq!(r, Ok("value".into()));
    assert_eq!(take_log(), ["exit", "returnValue value"]);
    assert!(cx.exit_context().is_none());
}

/// E8: `f <- function() { on.exit(tryCatch(stop("inner"), error = ...));
/// stop("outer") }; tryCatch(f(), error = conditionMessage)` is "outer".
#[test]
fn e8_an_on_exit_that_catches_its_own_error_returns_the_outer_condition() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let inner = l!(cx, "seq", l!(cx, "log", string(cx, "on.exit")), l!(cx, "catch", l!(cx, "stop", string(cx, "inner"))), l!(cx, "peek"));
        let f = l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", inner), l!(cx, "stop", string(cx, "outer"))));
        l!(cx, "catch", f)
    });
    assert_eq!(r, Ok("outer".into()));
    // (an intermediate stop: returnValue is undefined while on.exit runs)
    assert_eq!(take_log(), ["on.exit", "returnValue undefined"]);
    assert_eq!(cx.pending_exits(), 0);
    assert_eq!(cx.context_depth(), 1);
}

/// `function() { on.exit(return("five")); stop("boom") }` returns "five":
/// the `return` lands in the record whose on.exit code runs, and the error
/// it was unwinding is gone.
#[test]
fn a_return_in_on_exit_supersedes_the_error() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let f = l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", l!(cx, "return", string(cx, "five"))), l!(cx, "stop", string(cx, "boom"))));
        l!(cx, "catch", f)
    });
    assert_eq!(r, Ok("five".into()));
    assert_eq!(cx.pending_exits(), 0);
}

/// An error in the first on.exit expression: the rest still runs (the
/// record is the error's first stop), and that error is what leaves.
#[test]
fn an_error_in_on_exit_leaves_and_the_rest_still_runs() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let body = l!(
            cx,
            "seq",
            l!(cx, "onexit", l!(cx, "stop", string(cx, "A"))),
            l!(cx, "onexit", l!(cx, "log", string(cx, "B")), string(cx, "T")),
            string(cx, "one")
        );
        l!(cx, "catch", l!(cx, "fn", body))
    });
    assert_eq!(r, Ok("A".into()));
    assert_eq!(take_log(), ["B"]);
}

/// `on.exit(add = TRUE, after = FALSE)` runs first; `add = FALSE`
/// replaces; a NULL `expr` with `add = FALSE` clears.
#[test]
fn on_exit_add_and_after() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let body = l!(
            cx,
            "seq",
            l!(cx, "onexit", l!(cx, "log", string(cx, "replaced"))),
            l!(cx, "onexit", l!(cx, "log", string(cx, "A"))),
            l!(cx, "onexit", l!(cx, "log", string(cx, "B")), string(cx, "T")),
            l!(cx, "onexit", l!(cx, "log", string(cx, "first")), string(cx, "T"), string(cx, "F")),
            string(cx, "done")
        );
        l!(cx, "fn", body)
    });
    assert_eq!(r, Ok("done".into()));
    assert_eq!(take_log(), ["first", "A", "B"]);
}

/// An error crossing two closures with on.exit code: innermost first, the
/// catcher gets the error.
#[test]
fn an_error_runs_on_exit_code_innermost_first() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let g = l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", l!(cx, "log", string(cx, "g"))), l!(cx, "stop", string(cx, "deep"))));
        let f = l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", l!(cx, "log", string(cx, "f"))), g));
        l!(cx, "catch", f)
    });
    assert_eq!(r, Ok("deep".into()));
    assert_eq!(take_log(), ["g", "f"]);
}

/// `return(x)` from a nested closure's body lands in that closure only.
#[test]
fn return_lands_in_its_own_function() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let g = l!(cx, "fn", l!(cx, "seq", l!(cx, "return", string(cx, "from g")), l!(cx, "log", string(cx, "unreached"))));
        l!(cx, "fn", l!(cx, "seq", l!(cx, "log", g), string(cx, "f's value")))
    });
    assert_eq!(r, Ok("f's value".into()));
    assert_eq!(take_log(), ["from g"]);
}

/// An error that reaches the top level unwinds every record.
#[test]
fn an_uncaught_error_reaches_the_top_level() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", l!(cx, "log", string(cx, "cleanup"))), l!(cx, "stop", string(cx, "top")))));
    assert_eq!(r, Err("top".into()));
    assert_eq!(take_log(), ["cleanup"]);
    assert_eq!(cx.context_depth(), 1);
}

// ---- the chain is a precise root -------------------------------------------

/// Objects only a record holds survive a collection: the on.exit code (its
/// list and expressions), the body's value while on.exit runs, and an
/// exit parked in a record while its on.exit code runs.
#[test]
fn records_and_parked_exits_are_roots() {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, max_vsize: Some(80 << 20) });
    ISO.set(iso.cx().call_out(|re| re.as_ptr()));
    take_log();
    let cx = iso.cx();
    // the body's value and the on.exit code are reachable only from f's
    // record while (gc) runs inside the on.exit code
    let r = run(cx, |cx| {
        let code = l!(cx, "seq", l!(cx, "gc"), l!(cx, "peek"), l!(cx, "log", string(cx, "on.exit code kept")));
        l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", code), string(cx, "the value")))
    });
    assert_eq!(r, Ok("the value".into()));
    assert_eq!(take_log(), ["returnValue the value", "on.exit code kept"]);
    // the error's message is reachable only from the record it is parked in
    let r = run(cx, |cx| {
        let f = l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", l!(cx, "gc")), l!(cx, "stop", string(cx, "parked message"))));
        l!(cx, "catch", f)
    });
    assert_eq!(r, Ok("parked message".into()));
}

/// `for_each_root` lists what a record holds.
#[test]
fn the_root_list_includes_the_chain() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let call = s.root(cx, string(cx, "the call"));
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext { callflag: ctxt::RETURN, call, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
        let want = call.bind(cx).sexp();
        let out = s.out(cx);
        cx.function_context(&spec, out, &mut eval_discard, |cx, _, out| {
            let mut roots: Vec<Sexp> = Vec::new();
            cx.for_each_root(&mut |x| roots.push(x));
            assert!(roots.contains(&want));
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
}

// ---- jumps --------------------------------------------------------------------

#[test]
fn findcontext_and_r_jumptocontext_say_what_gnu_r_says() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let e = cx.find_context(ctxt::FUNCTION, rho, None);
        assert_eq!(text(cx.exit_objects(&e).0), "no function to return from, jumping to top level");
        cx.resolve_exit(e, None);
        let e = cx.find_context(ctxt::BREAK, rho, None);
        assert_eq!(text(cx.exit_objects(&e).0), "no loop for break/next, jumping to top level");
        cx.resolve_exit(e, None);
        // a record that is gone
        let mut gone = None;
        let nil = s.root(cx, cx.nil());
        let _ = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |_, id| {
            gone = Some(id);
            Ok(())
        });
        let e = cx.jump_to_context(gone.unwrap(), ctxt::BREAK, None);
        assert_eq!(text(cx.exit_objects(&e).0), "target context is not on the stack");
        cx.resolve_exit(e, None);
    });
}

/// A loop record lands `break` and `next` (`findcontext` with a loop mask)
/// and reports their mask; the value goes to the `Out` given.
#[test]
fn loop_records_land_break_and_next() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        for mask in [ctxt::BREAK, ctxt::NEXT] {
            let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> {
                // a closure in between: break crosses it
                cx.scope(|cx, s| {
                    let fenv = s.root(cx, env(cx));
                    let spec = FunctionContext { callflag: ctxt::RETURN, call: nil, cloenv: fenv, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
                    let out = s.out(cx);
                    cx.function_context(&spec, out, &mut eval_discard, |cx, _, _| Err(cx.find_context(mask, rho, None)))
                })
            });
            match r {
                Err(Left::Landed(info)) => assert_eq!(info.mask, mask),
                _ => panic!("the loop did not land"),
            }
        }
        assert_eq!(cx.pending_exits(), 0);
    });
}

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
    unwind_protect: extern "C" fn(*const c_void, *mut c_void, unsafe extern "C" fn(*mut c_void, c_int), *mut c_void, Sexp) -> Sexp,
    continue_unwind: extern "C" fn(Sexp),
    error_here: extern "C" fn(*const c_char),
    look: extern "C" fn(),
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
};

unsafe extern "C" {
    fn rt_test_cend_case(h: *const Hooks) -> Sexp;
    fn rt_test_cend_raises(h: *const Hooks) -> Sexp;
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

extern "C" fn h_error(msg: *const c_char) {
    let msg = cstr(msg);
    export(|cx| {
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
    let msg = cstr(msg);
    export(|cx| Err::<(), _>(cx.error(&msg)))
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
        let spec = FunctionContext { callflag: ctxt::RETURN, call: fcall, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
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
        let catcher = FunctionContext { callflag: ctxt::RETURN, call: nil, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
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
            let spec = FunctionContext { callflag: ctxt::RETURN, call: nil, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
            let out = s.out(cx);
            let res = out.root();
            let mut d = ExecData { h: &HOOKS, value: value.bind(cx).sexp(), fail, cleanups: 0 };
            let r = cx.function_context(&spec, out, &mut eval_discard, |cx, id, out| {
                CATCHERS.with_borrow_mut(|c| c.push(id));
                let r = unsafe { cx.exec_with_cleanup(nil, rt_test_exec_fun as *const c_void, (&raw mut d).cast(), rt_test_exec_cleanup, (&raw mut d).cast(), out) };
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
        let spec = FunctionContext { callflag: ctxt::RETURN, call, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
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
        let spec = FunctionContext { callflag: ctxt::RETURN, call, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
        let out = s.out(cx);
        cx.function_context(&spec, out, &mut eval_discard, |cx, _, out| {
            let depth = cx.context_depth();
            unsafe { cx.call_native6(rt_test_look as *const c_void, [hooks() as usize, 0, 0, 0, 0, 0], Ret6::Word) }?;
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
    assert_eq!(unsafe { cx.call_native6(rt_test_half as *const c_void, a, Ret6::Double) }.unwrap(), Native6::Double(21.5));
    assert_eq!(unsafe { cx.call_native6(rt_test_cplx as *const c_void, a, Ret6::Complex) }.unwrap(), Native6::Complex([40.0, 3.0]));
}

// ---- frame walks ---------------------------------------------------------------

/// `f(1)` called from the top, `g(2, 3)` called from `f`'s frame, `h()`
/// called from the top level's frame inside `g` (as `eval(quote(h()),
/// globalenv())` would): the walks GNU R's `sys.*`, `parent.frame` and
/// `nargs` use.
#[test]
fn frame_walks_count_as_gnu_r_does() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let global = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let (fenv, genv, henv) = (s.root(cx, env(cx)), s.root(cx, env(cx)), s.root(cx, env(cx)));
        let (fcall, gcall, hcall) = (s.root(cx, string(cx, "f(1)")), s.root(cx, string(cx, "g(2, 3)")), s.root(cx, string(cx, "h()")));
        let one = s.root(cx, cx.cons(cx.nil(), cx.nil()).unwrap());
        let two = s.root(cx, cx.cons(cx.nil(), one.bind(cx)).unwrap());
        let spec = |call, cloenv, sysparent, promargs| FunctionContext { callflag: ctxt::RETURN, call, cloenv, sysparent, callfun: nil, promargs };
        let fspec = spec(fcall, fenv, global, Promargs::Filled(one));
        let gspec = spec(gcall, genv, fenv, Promargs::Filled(two));
        let hspec = spec(hcall, henv, global, Promargs::Lazy { nargs: 0, fill: fill_nil, data: 0 });
        let out = s.out(cx);
        cx.function_context(&fspec, out, &mut eval_discard, |cx, _, out| {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                cx.function_context(&gspec, o, &mut eval_discard, |cx, _, o| {
                    cx.scope(|cx, s| {
                        let o2 = s.out(cx);
                        cx.function_context(&hspec, o2, &mut eval_discard, |cx, hid, o2| {
                            let g = global.bind(cx);
                            let top = cx.global_context();
                            assert_eq!(cx.framedepth(top), 3);
                            let call = |n| text(cx.sys_call_context(n, top).unwrap().call());
                            assert_eq!(call(0), "h()");
                            assert_eq!(call(-1), "g(2, 3)");
                            assert_eq!(call(1), "f(1)");
                            assert_eq!(cx.sys_call_context(4, top).unwrap_err(), WalkError::NotThatMany);
                            assert!(matches!(cx.sys_frame(0, top), Ok(Frame::Global)));
                            assert!(matches!(cx.sys_frame(2, top), Ok(Frame::Env(e)) if e == genv.bind(cx)));
                            assert_eq!(cx.sys_frame(i32::MIN, top).unwrap_err(), WalkError::NaArgument);
                            // h was called from the global frame; g from f's (frame 1)
                            assert_eq!(cx.sys_parent(1, top, g), Ok(0));
                            assert_eq!(cx.sys_parent(2, top, g), Ok(1));
                            assert_eq!(cx.sys_parent(0, top, g), Err(WalkError::OnlyPositive));
                            // h was called from the global frame, which no
                            // record has: R_GlobalEnv
                            assert!(matches!(cx.parent_frame(1), Ok(Frame::Global)));
                            assert_eq!(cx.parent_frame(0).unwrap_err(), WalkError::InvalidValue("n"));
                            assert_eq!(cx.nargs(fenv.bind(cx)), 1);
                            assert_eq!(cx.nargs(genv.bind(cx)), 2);
                            assert_eq!(cx.nargs(env(cx)), i32::MIN, "NA outside a function");
                            // h's promargs is lazy: counted without making it,
                            // made once when read
                            assert_eq!(cx.nargs(henv.bind(cx)), 0);
                            assert!(cx.context(hid).unwrap().promargs().is_none());
                            assert_eq!(FILLS.get(), 0);
                            cx.materialize_promargs(hid)?;
                            cx.materialize_promargs(hid)?;
                            assert_eq!(FILLS.get(), 1);
                            assert_eq!(cx.context(hid).unwrap().promargs(), Some(cx.nil()));
                            assert!(matches!(cx.current_env(), Frame::Env(e) if e == henv.bind(cx)));
                            let e = cx.walk_error(WalkError::NotThatMany);
                            let (msg, _, call) = cx.exit_objects(&e);
                            assert_eq!((text(msg), text(call)), ("not that many frames on the stack".into(), "h()".into()));
                            cx.resolve_exit(e, None);
                            // parent.frame() called in g's frame (its own
                            // record, as the closure parent.frame makes):
                            // g was called from f's frame
                            cx.scope(|cx, s| {
                                let penv = s.root(cx, env(cx));
                                let pcall = s.root(cx, string(cx, "parent.frame()"));
                                let pspec = FunctionContext { callflag: ctxt::RETURN, call: pcall, cloenv: penv, sysparent: genv, callfun: nil, promargs: Promargs::Filled(nil) };
                                let o3 = s.out(cx);
                                cx.function_context(&pspec, o3, &mut eval_discard, |cx, _, o3| {
                                    assert!(matches!(cx.parent_frame(1), Ok(Frame::Env(e)) if e == fenv.bind(cx)));
                                    // f was called from the global frame
                                    // (its sysparent, read from f's record)
                                    assert!(matches!(cx.parent_frame(2), Ok(Frame::Env(e)) if e == global.bind(cx)));
                                    assert!(matches!(cx.parent_frame(3), Ok(Frame::Global)));
                                    // do_sys counts from the frame sys.* was called from (g's)
                                    assert_eq!(text(cx.sys_context().call()), "g(2, 3)");
                                    assert_eq!(cx.sys_parent_n(1, global.bind(cx)), Ok(1));
                                    o3.set(cx, cx.nil());
                                    Ok(())
                                })
                            })?;
                            o2.set(cx, cx.nil());
                            Ok(())
                        })
                    })?;
                    o.set(cx, cx.nil());
                    Ok(())
                })
            })?;
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
}

fn fill_nil(cx: &mut Cx, _: ContextId, _: usize, out: Out<'_>) -> RResult<()> {
    FILLS.set(FILLS.get() + 1);
    out.set(cx, cx.nil());
    Ok(())
}

/// `sys.on.exit()`: NULL, the one expression, or `{` around several.
#[test]
fn sys_on_exit_shows_the_code_as_one_expression() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let a = s.root(cx, string(cx, "A"));
        let b = s.root(cx, string(cx, "B"));
        let spec = FunctionContext { callflag: ctxt::RETURN, call: nil, cloenv: rho, sysparent: rho, callfun: nil, promargs: Promargs::Filled(nil) };
        let out = s.out(cx);
        let show = |cx: &mut Cx, id| {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                let r = o.root();
                cx.sys_on_exit(id, o).unwrap();
                let x = r.bind(cx);
                match x.sexptype() {
                    LANGSXP => format!("{{ {} }}", args_of(x).1.iter().map(|&a| text(a)).collect::<Vec<_>>().join("; ")),
                    _ => text(x),
                }
            })
        };
        cx.function_context(&spec, out, &mut |_, _, _| Ok(()), |cx, id, out| {
            assert_eq!(show(cx, id), "NULL");
            cx.on_exit(nil, rho, a, None, None)?;
            assert_eq!(show(cx, id), "A");
            cx.on_exit(nil, rho, b, Some(1), None)?;
            assert_eq!(show(cx, id), "{ A; B }");
            // NA add is GNU R's error against on.exit's call
            let e = cx.on_exit(nil, rho, b, Some(i32::MIN), None).unwrap_err();
            assert_eq!(text(cx.exit_objects(&e).0), "invalid 'add' argument");
            cx.resolve_exit(e, None);
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
}

// ---- limits ----------------------------------------------------------------------

#[test]
fn the_evaluation_depth_limit_says_what_gnu_r_says() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_expressions(5);
    let mut saved = Vec::new();
    for _ in 0..5 {
        saved.push(cx.eval_depth_enter().unwrap());
    }
    let e = cx.eval_depth_enter().unwrap_err();
    let (msg, _, call) = cx.exit_objects(&e);
    assert_eq!(text(msg), robigo_rtcore::EXPR_SO_MSG);
    assert_eq!(call.sexptype(), NILSXP);
    assert_eq!(cx.expressions(), 505, "R_Expressions_keep + 500 while the error is handled");
    cx.resolve_exit(e, None);
    for s in saved.into_iter().rev() {
        cx.eval_depth_leave(s);
    }
    assert_eq!(cx.eval_depth(), 0);
}

/// The depth and the limit come back at an exit's stop
/// (`R_restore_globals`).
#[test]
fn an_exits_stop_restores_the_depth_and_the_limit() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_expressions(5);
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> {
            loop {
                cx.eval_depth_enter()?;
            }
        });
        let Err(Left::Exit(e)) = r else { panic!("no error") };
        // the error passed the loop record: its stop is the session's base
        assert_eq!(cx.eval_depth(), 6);
        cx.resolve_exit(e, None);
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> {
            for _ in 0..3 {
                cx.eval_depth_enter()?;
            }
            Err(cx.find_context(ctxt::BREAK, rho, None))
        });
        assert!(matches!(r, Err(Left::Landed(_))));
        assert_eq!(cx.eval_depth(), 6, "the loop record saved the depth it began at");
        assert_eq!(cx.expressions(), 5);
    });
}

/// `R_CheckStack()`: deep enough recursion is GNU R's error, the limit is
/// raised by 5% while it is handled, and an exit's stop restores it.
#[test]
fn the_c_stack_limit_says_what_gnu_r_says() {
    fn deep(cx: &mut Cx, n: usize) -> RResult<usize> {
        let pad = [0u8; 2048];
        std::hint::black_box(&pad);
        cx.check_c_stack()?;
        deep(cx, n + 1).map(|d| d + std::hint::black_box(pad[0]) as usize)
    }
    let mut iso = iso();
    let cx = iso.cx();
    let (limit, _, dir) = cx.c_stack_info();
    let Some(limit) = limit else {
        eprintln!("skipped: no C stack limit");
        return;
    };
    assert_eq!(dir, 1);
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| deep(cx, 0).map(|_| ()));
        let Err(Left::Exit(e)) = r else { panic!("no error") };
        let msg = text(cx.exit_objects(&e).0);
        assert!(msg.starts_with("C stack usage  ") && msg.ends_with(" is too close to the limit"), "{msg}");
        assert!(cx.c_stack_info().0.unwrap() > limit, "raised while the error is handled");
        cx.resolve_exit(e, None);
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> { Err(cx.find_context(ctxt::BREAK, rho, None)) });
        assert!(matches!(r, Err(Left::Landed(_))));
        assert_eq!(cx.c_stack_info().0, Some(limit), "an exit's stop restores it");
    });
}

#[test]
fn the_value_stack_check_is_gnu_rs_node_stack_overflow() {
    let mut iso = iso();
    let cx = iso.cx();
    assert!(cx.check_value_stack(10).is_ok());
    let e = cx.check_value_stack(cx.stack_headroom() + 1).unwrap_err();
    assert_eq!(text(cx.exit_objects(&e).0), robigo_rtcore::NODE_SO_MSG);
    cx.resolve_exit(e, None);
}

#[test]
fn a_pending_interrupt_waits_while_suspended() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_interrupt_pending();
    cx.set_interrupts_suspended(true);
    assert!(cx.check_user_interrupt().is_ok());
    cx.set_interrupts_suspended(false);
    let e = cx.check_user_interrupt().unwrap_err();
    assert_eq!(cx.exit_info(&e).kind, ExitKind::Interrupt);
    assert_eq!(cx.exit_info(&e).target, cx.toplevel_context().id().addr());
    cx.resolve_exit(e, None);
    assert!(cx.check_user_interrupt().is_ok(), "taken once");
}
