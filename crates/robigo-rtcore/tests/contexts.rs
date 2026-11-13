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

mod common;
use common::*;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Context, ContextId, Cx, ExitKind, Frame, FunctionContext, Left, Out, Promargs, RResult, Sexp, WalkError, ctxt,
};
use std::cell::RefCell;

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
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call: nil,
            cloenv: rho,
            sysparent: rho,
            callfun: fun,
            promargs: Promargs::Filled(nil),
        };
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
        let inner = l!(
            cx,
            "seq",
            l!(cx, "log", string(cx, "on.exit")),
            l!(cx, "catch", l!(cx, "stop", string(cx, "inner"))),
            l!(cx, "peek")
        );
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
        let f = l!(
            cx,
            "fn",
            l!(cx, "seq", l!(cx, "onexit", l!(cx, "return", string(cx, "five"))), l!(cx, "stop", string(cx, "boom")))
        );
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
        let g = l!(
            cx,
            "fn",
            l!(cx, "seq", l!(cx, "onexit", l!(cx, "log", string(cx, "g"))), l!(cx, "stop", string(cx, "deep")))
        );
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
        let g =
            l!(cx, "fn", l!(cx, "seq", l!(cx, "return", string(cx, "from g")), l!(cx, "log", string(cx, "unreached"))));
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
    let r = run(cx, |cx| {
        l!(
            cx,
            "fn",
            l!(cx, "seq", l!(cx, "onexit", l!(cx, "log", string(cx, "cleanup"))), l!(cx, "stop", string(cx, "top")))
        )
    });
    assert_eq!(r, Err("top".into()));
    assert_eq!(take_log(), ["cleanup"]);
    assert_eq!(cx.context_depth(), 1);
}

// ---- the chain is a precise root -------------------------------------------

/// Objects only a record holds survive a collection: the body's value
/// while on.exit code runs (`returnValue`), an exit parked in a record while
/// its on.exit code runs, and the `on.exit` list (cells `on.exit` made).
#[cfg(not(miri))]
#[test]
fn records_and_parked_exits_are_roots() {
    let mut iso = iso();
    let cx = iso.cx();
    let r = run(cx, |cx| {
        let code = l!(cx, "seq", l!(cx, "gc"), l!(cx, "peek"));
        l!(cx, "fn", l!(cx, "seq", l!(cx, "onexit", code), l!(cx, "fresh", string(cx, "the value"))))
    });
    assert_eq!(r, Ok("the value".into()));
    assert_eq!(take_log(), ["returnValue the value"]);
    let r = run(cx, |cx| {
        let f = l!(
            cx,
            "fn",
            l!(
                cx,
                "seq",
                l!(cx, "onexit", l!(cx, "gc")),
                l!(cx, "stop", l!(cx, "fresh", string(cx, "parked message")))
            )
        );
        l!(cx, "catch", f)
    });
    assert_eq!(r, Ok("parked message".into()));
    // the list's cells: a (gc) in the body, then the list runs
    let r = run(cx, |cx| {
        let body = l!(
            cx,
            "seq",
            l!(cx, "onexit", l!(cx, "log", string(cx, "first"))),
            l!(cx, "onexit", l!(cx, "log", string(cx, "second")), string(cx, "T")),
            l!(cx, "gc"),
            string(cx, "done")
        );
        l!(cx, "fn", body)
    });
    assert_eq!(r, Ok("done".into()));
    assert_eq!(take_log(), ["first", "second"]);
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
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
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
                    let spec = FunctionContext {
                        callflag: ctxt::RETURN,
                        call: nil,
                        cloenv: fenv,
                        sysparent: rho,
                        callfun: nil,
                        promargs: Promargs::Filled(nil),
                    };
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
        let (fcall, gcall, hcall) =
            (s.root(cx, string(cx, "f(1)")), s.root(cx, string(cx, "g(2, 3)")), s.root(cx, string(cx, "h()")));
        let one = s.root(cx, cx.cons(cx.nil(), cx.nil()).unwrap());
        let two = s.root(cx, cx.cons(cx.nil(), one.bind(cx)).unwrap());
        let spec = |call, cloenv, sysparent, promargs| FunctionContext {
            callflag: ctxt::RETURN,
            call,
            cloenv,
            sysparent,
            callfun: nil,
            promargs,
        };
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
                            assert_eq!(
                                (text(msg), text(call)),
                                ("not that many frames on the stack".into(), "h()".into())
                            );
                            cx.resolve_exit(e, None);
                            // parent.frame() called in g's frame (its own
                            // record, as the closure parent.frame makes):
                            // g was called from f's frame
                            cx.scope(|cx, s| {
                                let penv = s.root(cx, env(cx));
                                let pcall = s.root(cx, string(cx, "parent.frame()"));
                                let pspec = FunctionContext {
                                    callflag: ctxt::RETURN,
                                    call: pcall,
                                    cloenv: penv,
                                    sysparent: genv,
                                    callfun: nil,
                                    promargs: Promargs::Filled(nil),
                                };
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
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call: nil,
            cloenv: rho,
            sysparent: rho,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let out = s.out(cx);
        let show = |cx: &mut Cx, id| {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                let r = o.root();
                cx.sys_on_exit(id, o).unwrap();
                let x = r.bind(cx);
                match x.sexptype() {
                    LANGSXP => {
                        format!("{{ {} }}", args_of(x).1.iter().map(|&a| text(a)).collect::<Vec<_>>().join("; "))
                    }
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
    fn deep(cx: &mut Cx, depth: usize) -> RResult<usize> {
        let pad = [0u8; 2048];
        std::hint::black_box(&pad);
        cx.check_c_stack()?;
        deep(cx, depth + 1).map(|d| d.max(depth) + std::hint::black_box(pad[0]) as usize)
    }
    let mut iso = iso();
    let cx = iso.cx();
    let info = cx.c_stack_info();
    let Some(limit) = info.limit else {
        eprintln!("skipped: no C stack limit");
        return;
    };
    assert_eq!(info.dir, 1);
    // an `extra` past isize::MAX is "too much", not a wrapped negative
    let e = cx.check_c_stack_extra(usize::MAX).unwrap_err();
    assert!(text(cx.exit_objects(&e).0).starts_with("C stack usage  "));
    cx.resolve_exit(e, None);
    assert_eq!(cx.c_stack_info().limit, Some((limit as f64 / 0.95) as usize));
    let r = cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> {
            Err(cx.find_context(ctxt::BREAK, rho, None))
        })
    });
    assert!(matches!(r, Err(Left::Landed(_))));
    assert_eq!(cx.c_stack_info().limit, Some(limit));
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let nil = s.root(cx, cx.nil());
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| deep(cx, 0).map(|_| ()));
        let Err(Left::Exit(e)) = r else { panic!("no error") };
        let msg = text(cx.exit_objects(&e).0);
        assert!(msg.starts_with("C stack usage  ") && msg.ends_with(" is too close to the limit"), "{msg}");
        // R_SignalCStackOverflow: R_CStackLimit / 0.95 while it is handled
        assert_eq!(cx.c_stack_info().limit, Some((limit as f64 / 0.95) as usize));
        cx.resolve_exit(e, None);
        let r = cx.with_context(ctxt::LOOP, nil, rho, rho, None, |cx, _| -> RResult<()> {
            Err(cx.find_context(ctxt::BREAK, rho, None))
        });
        assert!(matches!(r, Err(Left::Landed(_))));
        assert_eq!(cx.c_stack_info().limit, Some(limit), "an exit's stop restores it");
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

/// The raw layer's function record behaves as the safe one: a `return` to
/// it lands, on.exit code runs, the value reaches the caller's slot.
#[test]
fn the_raw_function_record_lands_returns() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let rho = s.root(cx, env(cx));
        let code = s.root(cx, l!(cx, "log", string(cx, "raw on.exit")));
        let v = s.root(cx, string(cx, "returned"));
        let x = |r: robigo_rtcore::Root<'_>, cx: &Cx| r.bind(cx).sexp();
        let spec = robigo_rtcore::RawFunctionContext {
            callflag: ctxt::RETURN,
            call: x(rho, cx),
            cloenv: x(rho, cx),
            sysparent: x(rho, cx),
            callfun: x(rho, cx),
            promargs: Ok(cx.nil().sexp()),
        };
        let out = s.out(cx);
        let res = out.root();
        unsafe {
            cx.function_context_raw(&spec, out, &mut eval_discard, |cx, _, out| {
                cx.on_exit(rho, rho, code, None, None)?;
                out.set(cx, cx.nil());
                Err(cx.find_context(ctxt::FUNCTION, rho, Some(v)))
            })
        }
        .unwrap();
        assert_eq!(text(res.bind(cx)), "returned");
    });
    assert_eq!(take_log(), ["raw on.exit"]);
}
