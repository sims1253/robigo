//! S3 dispatch's argument aliases (`env::alias`) under E9's injected
//! allocation failure (`ROBIGO_FAIL_ALLOC_AT`, read when an isolate is
//! made; docs/g3.md §15.3): the promises a method's call's end makes of
//! its aliases come from a reserve made with the aliases
//! (`Cx::reserve_alias_promises`), whose refused allocation is retried as
//! any is, and the call's end allocates nothing. Its own test binary, so
//! that the knob reaches no other test.

#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::{
    BindingValue, Cx, FunctionContext, Lookup, OwnedIsolate, Promargs, RResult, Root, Scope, SlotInit, View, ctxt,
};

fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &str) -> Root<'s> {
    s.root(cx, cx.install(name.as_bytes()).unwrap())
}

/// The value bound to `name` in the frame `env`.
fn value<'g>(cx: &'g Cx, env: Root<'_>, name: &str) -> robigo_rtcore::Ref<'g> {
    let s = cx.install(name.as_bytes()).unwrap();
    let View::Env(e) = env.bind(cx).view() else { panic!("not an environment") };
    match cx.frame_binding(e, s) {
        Lookup::Bound(b) => match b.get(cx) {
            BindingValue::Value(v) => v,
            v => panic!("{name}: not a value: {v:?}"),
        },
        _ => panic!("{name}: unbound"),
    }
}

/// Run `body` in a function record of frame `f` (the call begun and ended
/// around it, as a return).
fn in_call(cx: &mut Cx, f: Root<'_>, body: impl FnOnce(&mut Cx) -> RResult<()>) {
    cx.begin_call(f);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let nil = s.root(cx, cx.nil());
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call: nil,
            cloenv: f,
            sysparent: g,
            callfun: nil,
            promargs: Promargs::Filled(nil),
        };
        let mut no_on_exit = |_: &mut Cx, _: Root<'_>, _: Root<'_>| Ok(());
        let out = s.out(cx);
        cx.function_context(&spec, out, &mut no_on_exit, |cx, _, out| {
            body(cx)?;
            out.set(cx, cx.nil());
            Ok(())
        })
        .unwrap();
    });
    cx.end_call(f, true);
}

// (a refused allocation is retried after a collection, which Miri does
// not run: tests/frames.rs runs the call's end under it)
#[cfg(not(miri))]
#[test]
fn a_methods_call_ends_without_allocating() {
    // SAFETY: this binary's one test sets it before any isolate is made,
    // and nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("ROBIGO_FAIL_ALLOC_AT", "1") };
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let syms = [cx.install(b"x").unwrap(), cx.install(b"y").unwrap()];
        let sh = s.root(cx, cx.new_shape(&syms).unwrap());
        let expr = sym(cx, s, "some_expr");
        let five = s.root(cx, cx.alloc_copy(&[5.0]).unwrap().finish());
        let g = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let generic = out.root();
        let inits = [SlotInit::Delayed { expr, value: None, bytecode: false }, SlotInit::Forced { expr, value: five }];
        cx.new_closure_frame(sh, g, g, &inits, out).unwrap();
        let out = s.out(cx);
        let method = out.root();
        let mut made = Some(out);
        in_call(cx, generic, |cx| {
            // the reserve's allocation, refused once: retried after a
            // collection, as every refused allocation is
            let before = cx.gc_stats().collections;
            cx.arm_fail_alloc();
            cx.reserve_alias_promises(2)?;
            assert_eq!(cx.gc_stats().collections, before + 1, "the reserve's refused allocation was not retried");
            let aliases = [SlotInit::Alias { frame: generic, index: 0 }, SlotInit::Alias { frame: generic, index: 1 }];
            cx.new_closure_frame(sh, g, g, &aliases, made.take().unwrap())?;
            // the method's call left by a jump (no cleanup): both aliases
            // become the generic's promises, and the next allocation, armed
            // to be refused, is not reached
            cx.begin_call(method);
            let before = cx.gc_stats().collections;
            cx.arm_fail_alloc();
            cx.end_call(method, false);
            assert_eq!(cx.gc_stats().collections, before, "the call's end allocated");
            for name in ["x", "y"] {
                let p = value(cx, method, name);
                assert!(matches!(p.view(), View::Promise(_)), "{name}: not a promise");
                assert_eq!(p.sexp(), value(cx, generic, name).sexp(), "{name}: not the generic's promise");
            }
            Ok(())
        });
    });
}
