//! S3 dispatch's argument aliases (`env::alias`) under D5's quarantine
//! mode (`ROBIGO_FRAME_QUARANTINE=1`, read when an isolate is made): the
//! generic's frame, recycled once its call is over, is poisoned (no longer
//! a slot frame), and its method's frame, which outlives its call, must
//! not read it any more. Its own test binary, so that the mode reaches no
//! other test.

#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::{
    ArgSlot, BindingValue, Cx, FunctionContext, Lookup, OwnedIsolate, Promargs, RResult, Root, Scope, SlotInit, View,
    ctxt,
};

fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &str) -> Root<'s> {
    s.root(cx, cx.install(name.as_bytes()).unwrap())
}

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

#[test]
fn a_methods_frame_outlives_its_generics_poisoned_frame() {
    // SAFETY: this binary's one test sets it before any isolate is made,
    // and nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("ROBIGO_FRAME_QUARANTINE", "1") };
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
            let aliases = [SlotInit::Alias { frame: generic, index: 0 }, SlotInit::Alias { frame: generic, index: 1 }];
            cx.reserve_alias_promises(aliases.len())?;
            cx.new_closure_frame(sh, g, g, &aliases, made.take().unwrap())?;
            // the method's call ends with its frame kept (no cleanup)
            in_call(cx, method, |_| Ok(()));
            Ok(())
        });
        // the generic's frame: cleaned and recycled, so poisoned
        cx.cleanup_envir(generic, None);
        cx.recycle_frame(generic, None);
        assert!(cx.slot_frame_shape(generic.bind(cx)).is_none(), "the generic's frame was not poisoned");
        // the method's frame holds the promises, and reads nothing of the
        // generic's frame
        let m = method.bind(cx).sexp();
        assert!(matches!(unsafe { cx.arg_slot_raw(m, 0) }, Some(ArgSlot::Promise(_))));
        assert!(unsafe { cx.frame_slot_raw(m, sh.bind(cx).sexp(), 1) }.is_some());
        let x = value(cx, method, "x");
        let View::Promise(px) = x.view() else { panic!("x: not a promise") };
        assert_eq!(px.value().sexp(), cx.unbound().sexp(), "x is still unforced");
        assert_eq!(px.code().sexp(), expr.bind(cx).sexp());
        let y = value(cx, method, "y");
        let View::Promise(py) = y.view() else { panic!("y: not a promise") };
        assert_eq!(py.value().sexp(), five.bind(cx).sexp());
    });
    // the poisoned frame, held as a root, counts no vector cells: the sweep
    // walks its block (garbage was made beside it), and in a debug build
    // checks each walked block's count against its objects' cells
    // (robigo-heap's `Block::cells`)
    for _ in 0..1000 {
        drop(cx.alloc_copy(&[1.0, 2.0, 3.0]).unwrap().finish());
    }
    cx.collect();
    cx.collect();
}
