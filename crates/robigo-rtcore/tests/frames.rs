//! Slot frames (N1b, docs/g3.md §5.1, §6.2-6.3): closure frames and every
//! unhashed environment, each rule against GNU R 4.6.2. The listing cases
//! replay `tests/oracle/envir-frames.R` (recorded with the 4.6.2 oracle
//! image) with frames made as a closure call makes them.

// Tests build closures and promises in the raw layer (their constructors
// are C1's) and read frame words to check what the API keeps private.
#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::types::*;
use robigo_rtcore::{
    BindingValue, Char, Cx, EnvRef, Lgl, Lookup, NeedGc, Options, Out, OwnedIsolate, RResult, Ref, Root, Scalar, Scope,
    SlotInit, SuperAssignCache, View,
};
use std::collections::HashMap;

fn oracle(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once(" : ").map(|(k, v)| (k.trim().to_owned(), v.trim_end().to_owned())))
        .collect()
}

fn iso() -> OwnedIsolate {
    OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() })
}

fn env_of<'g>(r: Ref<'g>) -> EnvRef<'g> {
    match r.view() {
        View::Env(e) => e,
        v => panic!("not an environment: {v:?}"),
    }
}

fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &str) -> Root<'s> {
    s.root(cx, cx.install(name.as_bytes()).unwrap())
}

fn num<'s>(cx: &mut Cx, s: Scope<'s>, x: f64) -> Root<'s> {
    s.root(cx, cx.alloc_copy(&[x]).unwrap().finish())
}

fn shape<'s>(cx: &mut Cx, s: Scope<'s>, names: &[&str]) -> Root<'s> {
    let syms: Vec<_> = names.iter().map(|n| cx.install(n.as_bytes()).unwrap()).collect();
    s.root(cx, cx.new_shape(&syms).unwrap())
}

/// A closure frame of `shape` called with `args`, enclosed by and called
/// from the global environment.
fn call_frame<'s>(cx: &mut Cx, s: Scope<'s>, shape: Root<'_>, args: &[SlotInit<'_>]) -> Root<'s> {
    let g = s.root(cx, cx.global_env());
    let out = s.out(cx);
    let r = out.root();
    cx.new_closure_frame(shape, g, g, args, out).unwrap();
    r
}

fn assign(cx: &mut Cx, name: &str, value: f64, env: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let k = sym(cx, s, name);
        let v = num(cx, s, value);
        cx.define_var(k, v, env)
    })
}

fn assign_obj(cx: &mut Cx, name: &str, value: Root<'_>, env: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let k = sym(cx, s, name);
        cx.define_var(k, value, env)
    })
}

fn rm(cx: &mut Cx, name: &str, env: Root<'_>) -> bool {
    cx.scope(|cx, s| {
        let k = sym(cx, s, name);
        cx.remove_variable(k, env).unwrap()
    })
}

fn strings(r: Ref<'_>) -> Vec<String> {
    r.as_vector::<Char<'_>>()
        .unwrap()
        .as_slice()
        .unwrap()
        .iter()
        .map(|c| String::from_utf8(c.bytes().to_vec()).unwrap())
        .collect()
}

fn ls(cx: &mut Cx, env: Root<'_>, all: bool) -> String {
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let r = out.root();
        cx.ls_internal(env, all, out).unwrap();
        strings(r.bind(cx)).join(" ")
    })
}

/// `get(name, envir, inherits = FALSE)`'s object, written to a root of `s`.
fn get_in<'s>(cx: &mut Cx, s: Scope<'s>, name: &str, env: Root<'_>) -> Root<'s> {
    let k = sym(cx, s, name);
    let out = s.out(cx);
    let r = out.root();
    cx.find_var_in_frame3(env, k, true, out).unwrap();
    r
}

fn real(r: Ref<'_>) -> f64 {
    r.as_vector::<f64>().unwrap().get(0)
}

fn msg<T: std::fmt::Debug>(cx: &mut Cx, r: RResult<T>) -> String {
    let e = r.expect_err("expected an error");
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let root = out.root();
        cx.resolve_exit(e, Some(out));
        strings(root.bind(cx)).join("")
    })
}

fn binding<'g>(cx: &'g Cx, env: Root<'_>, name: &str) -> Option<robigo_rtcore::Binding<'g>> {
    let s = cx.install(name.as_bytes()).unwrap();
    match cx.frame_binding(env_of(env.bind(cx)), s) {
        Lookup::Bound(b) => Some(b),
        _ => None,
    }
}

#[cfg(not(miri))]
fn collect(cx: &mut Cx) {
    cx.need_gc(NeedGc::new(INTSXP, 1)).unwrap();
}

/// A closure whose "body" is what calling it returns.
fn closure<'g>(cx: &'g Cx, body: Ref<'g>) -> Ref<'g> {
    let f = cx.lcons(cx.nil(), body).unwrap();
    unsafe { f.sexp().set_typ(CLOSXP) };
    f
}

/// The evaluator stand-in: `f()` returns the closure's body; a promise is
/// forced to its constant code (PRVALUE set).
fn test_eval(cx: &mut Cx, expr: Root<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    match expr.bind(cx).view() {
        View::Lang(c) => {
            let View::Closure(f) = c.car().view() else { panic!("not a closure call") };
            out.set(cx, f.body());
            Ok(())
        }
        View::Promise(p) => {
            let code = p.code();
            unsafe { p.as_ref().sexp().set_car(code.sexp()) };
            out.set(cx, code);
            Ok(())
        }
        v => panic!("test_eval: {v:?}"),
    }
}

// ---- listing order ----------------------------------------------------------

/// GNU R lists a closure frame newest first and its arguments last in
/// formal order; `rm` then reinsertion moves a binding to the front,
/// reassignment does not; a missing argument is bound; a local the shape
/// reserves but the call never defined is invisible (`exists` is FALSE).
/// envir-frames.R, `f(1)`.
#[test]
fn closure_frames_list_as_gnu_r() {
    let want = oracle(include_str!("oracle/envir-frames.out"));
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // function(a, b) { z <- 1; y <- 2; ... } with a local `w` it never sets
        let sh = shape(cx, s, &["a", "b", "z", "y", "w"]);
        let one = num(cx, s, 1.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one), SlotInit::Missing]);
        assign(cx, "z", 1.0, f).unwrap();
        assign(cx, "y", 2.0, f).unwrap();
        assert_eq!(ls(cx, f, false), want["F.ls"]);
        assert!(rm(cx, "z", f));
        assign(cx, "z", 3.0, f).unwrap();
        assert_eq!(ls(cx, f, false), want["F.re"]);
        assign(cx, "y", 4.0, f).unwrap();
        assert_eq!(ls(cx, f, false), want["F.upd"]);
        let w = sym(cx, s, "w");
        assert!(!cx.exists_var_in_frame(f, w).unwrap(), "F.exists.w");
        assert_eq!(binding(cx, f, "b").unwrap().missing(cx), 1, "F.missing.b");
        assert_eq!(ls(cx, f, true), want["F.aslist"]);
        // an unbound slot is skipped by lookups: `w` from the enclosure
        let g = s.root(cx, cx.global_env());
        assign(cx, "w", 8.0, g).unwrap();
        let out = s.out(cx);
        let r = out.root();
        cx.find_var(w, f, out).unwrap();
        assert_eq!(real(r.bind(cx)), 8.0);
    });
}

/// Names outside the shape go to the overflow block (`assign()`, a `for`
/// variable the compiler did not see, `delayedAssign`,
/// `makeActiveBinding`) and list among the shape's locals by when they
/// were defined, through the block's growth; `mget` reads both.
/// envir-frames.R, `g(7)` and `k()`.
#[test]
fn overflow_bindings_interleave_by_definition() {
    let want = oracle(include_str!("oracle/envir-frames.out"));
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "x"]);
        let seven = num(cx, s, 7.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(seven)]);
        assign(cx, "q", 1.0, f).unwrap();
        assign(cx, "x", 1.0, f).unwrap();
        assign(cx, "r", 2.0, f).unwrap();
        assert_eq!(ls(cx, f, false), want["G.ls"]);
        assign(cx, "n", 0.0, f).unwrap();
        for n in ["o1", "o2", "o3", "o4", "o5", "o6"] {
            assign(cx, n, 1.0, f).unwrap();
        }
        assert_eq!(ls(cx, f, false), want["G.many"]);
        rm(cx, "q", f);
        rm(cx, "o3", f);
        assign(cx, "q", 9.0, f).unwrap();
        assign(cx, "x", 5.0, f).unwrap();
        assert_eq!(ls(cx, f, false), want["G.re"]);
        let got: Vec<String> =
            ["x", "q", "a"].iter().map(|n| format!("{n} {}", real(get_in(cx, s, n, f).bind(cx)))).collect();
        assert_eq!(got.join(" "), want["G.mget"]);

        // k(): delayedAssign and makeActiveBinding outside the shape
        let sh = shape(cx, s, &["v"]);
        let k = call_frame(cx, s, sh, &[]);
        let nil = cx.nil();
        let p = cx.lcons(cx.unbound(), nil).unwrap();
        unsafe { p.sexp().set_typ(PROMSXP) };
        let p = s.root(cx, p);
        assign_obj(cx, "lazy", p, k).unwrap();
        let fun = s.root(cx, closure(cx, cx.nil()));
        let act = sym(cx, s, "act");
        cx.make_active_binding(act, fun, k).unwrap();
        assign(cx, "v", 1.0, k).unwrap();
        assert_eq!(ls(cx, k, false), want["K.ls"]);
        // new.env(hash = FALSE): no shape, every binding in the block
        let g = s.root(cx, cx.global_env());
        let e = s.root(cx, cx.new_env(g.bind(cx), false, 0).unwrap());
        for n in ["p1", "p2", "p3", "p4", "p5"] {
            assign(cx, n, 1.0, e).unwrap();
        }
        rm(cx, "p2", e);
        assign(cx, "p2", 2.0, e).unwrap();
        assert_eq!(ls(cx, e, false), want["E.ls"]);
        assert_eq!(ls(cx, e, true), want["E.aslist"]);
    });
}

/// `function(a, b = 2, ...)` called as `h(1)`: `b` a default (delayed in the
/// frame, MISSING 2), `...` missing; all list in formal order.
/// envir-frames.R, `h(1)`.
#[test]
fn arguments_of_every_kind_list_in_formal_order() {
    let want = oracle(include_str!("oracle/envir-frames.out"));
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "b", "..."]);
        let one = num(cx, s, 1.0);
        let two = num(cx, s, 2.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one), SlotInit::Default(two), SlotInit::Missing]);
        assert_eq!(ls(cx, f, true), want["H.ls"]);
        assert_eq!(binding(cx, f, "b").unwrap().missing(cx), 2);
        assert_eq!(binding(cx, f, "...").unwrap().missing(cx), 1);
        // reading `b` as an object materializes the promise, once
        let b1 = get_in(cx, s, "b", f);
        assert_eq!(b1.bind(cx).sexptype(), PROMSXP);
        let View::Promise(p) = b1.bind(cx).view() else { panic!() };
        assert_eq!(p.value(), cx.unbound(), "unforced");
        assert_eq!(p.env(), f.bind(cx), "a default evaluates in the frame");
        let b2 = get_in(cx, s, "b", f);
        assert_eq!(b1.bind(cx), b2.bind(cx), "one promise object for every reader");
        assert_eq!(binding(cx, f, "b").unwrap().missing(cx), 2, "MISSING stays with the binding");
        assert_eq!(ls(cx, f, true), want["H.after"]);
        // assigning clears MISSING
        assign(cx, "b", 3.0, f).unwrap();
        assert_eq!(binding(cx, f, "b").unwrap().missing(cx), 0);
    });
}

// ---- argument states ----------------------------------------------------------

/// A delayed argument reads as GNU R's unforced promise in the caller's
/// environment (with the caller's early value in its 4th word for C1's
/// force); a forced one as a forced promise; `findFun` forces a delayed one
/// through the evaluator and reads a forced one's value without a promise.
/// Once no delayed argument needs the caller, the frame drops it (GNU R
/// drops a forced promise's environment).
#[test]
fn delayed_and_forced_arguments_are_gnu_rs_promises() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let caller = s.root(cx, cx.new_env(g.bind(cx), false, 0).unwrap());
        let sh = shape(cx, s, &["d", "pre", "fo", "fn"]);
        let expr_d = sym(cx, s, "some_expr");
        let early = num(cx, s, 42.0);
        let val = num(cx, s, 5.0);
        let nil = cx.nil();
        let fun = s.root(cx, closure(cx, nil));
        let out = s.out(cx);
        let f = out.root();
        cx.new_closure_frame(
            sh,
            g,
            caller,
            &[
                SlotInit::Delayed { expr: expr_d, value: None },
                SlotInit::Delayed { expr: expr_d, value: Some(early) },
                SlotInit::Forced { expr: expr_d, value: val },
                SlotInit::Forced { expr: expr_d, value: fun },
            ],
            out,
        )
        .unwrap();
        let caller_word = |cx: &Cx| unsafe { f.bind(cx).sexp().sexp_at(robigo_heap::slots::OFF_CALLER) };
        assert_eq!(caller_word(cx), caller.bind(cx).sexp());
        assert!(matches!(binding(cx, f, "d").unwrap().get(cx), BindingValue::Deferred));
        // a forced argument's value is a function lookup's answer at once
        let fsym = sym(cx, s, "fn");
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(fsym, f, None, out).unwrap();
        assert_eq!(r.bind(cx), fun.bind(cx));
        assert!(matches!(binding(cx, f, "fn").unwrap().get(cx), BindingValue::Deferred), "not materialized");
        // the promises
        let d = get_in(cx, s, "d", f);
        let View::Promise(p) = d.bind(cx).view() else { panic!() };
        assert_eq!((p.value(), p.code(), p.env()), (cx.unbound(), expr_d.bind(cx), caller.bind(cx)));
        let pre = get_in(cx, s, "pre", f);
        let View::Promise(p) = pre.bind(cx).view() else { panic!() };
        assert_eq!(p.value(), cx.unbound(), "unforced until the callee's first read");
        assert_eq!(unsafe { p.as_ref().sexp().aux() }, early.bind(cx).sexp(), "the early value rides along");
        assert_eq!(caller_word(cx), cx.nil().sexp(), "no delayed argument needs the caller any more");
        let fo = get_in(cx, s, "fo", f);
        let View::Promise(p) = fo.bind(cx).view() else { panic!() };
        assert_eq!((p.value(), p.code(), p.env()), (val.bind(cx), expr_d.bind(cx), cx.nil()));
    });
}

/// `findFun` forces a delayed argument through the evaluator.
#[test]
fn find_fun_forces_a_delayed_argument() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let nil = cx.nil();
        let fun = s.root(cx, closure(cx, nil));
        let sh = shape(cx, s, &["h"]);
        // the "expression" is the closure itself (test_eval forces to it)
        let f = call_frame(cx, s, sh, &[SlotInit::Delayed { expr: fun, value: None }]);
        let h = sym(cx, s, "h");
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(h, f, None, out).unwrap();
        assert_eq!(r.bind(cx), fun.bind(cx));
        let _ = g;
    });
}

// ---- unboxed values --------------------------------------------------------

/// A scalar stored unboxed lists and reads like any value; reading it as
/// an object boxes it once and stores the box back, so every reader sees
/// one identity (`R_expand_binding_value`). A locked binding refuses it and
/// marks its box not mutable.
#[test]
fn unboxed_scalars_expand_once() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["i", "n", "l"]);
        let f = call_frame(cx, s, sh, &[]);
        let (i, n, l) = (sym(cx, s, "i"), sym(cx, s, "n"), sym(cx, s, "l"));
        cx.define_scalar(i, Scalar::Int(3), f).unwrap();
        cx.define_scalar(n, Scalar::Real(2.5), f).unwrap();
        cx.define_scalar(l, Scalar::Lgl(Lgl::TRUE), f).unwrap();
        assert_eq!(ls(cx, f, false), "l n i");
        assert!(matches!(binding(cx, f, "n").unwrap().get(cx), BindingValue::Deferred));
        let a = get_in(cx, s, "n", f);
        let b = get_in(cx, s, "n", f);
        assert_eq!(a.bind(cx), b.bind(cx), "boxed once");
        assert_eq!(real(a.bind(cx)), 2.5);
        assert_eq!(get_in(cx, s, "i", f).bind(cx).as_vector::<i32>().unwrap().get(0), 3);
        assert_eq!(get_in(cx, s, "l", f).bind(cx).as_vector::<Lgl>().unwrap().get(0), Lgl::TRUE);
        // the slot is reused unboxed by the next store
        cx.define_scalar(n, Scalar::Real(7.0), f).unwrap();
        assert!(matches!(binding(cx, f, "n").unwrap().get(cx), BindingValue::Deferred));
        assert_eq!(real(get_in(cx, s, "n", f).bind(cx)), 7.0);
        // locked: the store is GNU R's error; a read's box is not mutable
        cx.define_scalar(i, Scalar::Int(4), f).unwrap();
        cx.lock_binding(i, f).unwrap();
        let r = cx.define_scalar(i, Scalar::Int(5), f);
        assert_eq!(msg(cx, r), "cannot change value of locked binding for 'i'");
        assert!(get_in(cx, s, "i", f).bind(cx).maybe_shared());
        // a hashed frame takes it boxed
        let g = s.root(cx, cx.global_env());
        cx.define_scalar(n, Scalar::Real(1.5), g).unwrap();
        assert_eq!(real(get_in(cx, s, "n", g).bind(cx)), 1.5);
    });
}

// ---- cells -----------------------------------------------------------------

/// `R_findVarLocInFrame` makes a slot binding cell-backed for good: later
/// writes through the frame reach the cell, writes through the cell reach
/// the frame, the binding keeps its place in the listing, and `rm` detaches
/// the cell as GNU R detaches a removed cell (unbound, locked).
#[test]
fn a_varloc_makes_a_binding_cell_backed() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "v"]);
        let one = num(cx, s, 1.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one)]);
        assign(cx, "v", 2.0, f).unwrap();
        let a = sym(cx, s, "a");
        cx.define_scalar(a, Scalar::Real(1.0), f).unwrap();
        let v = sym(cx, s, "v");
        let out = s.out(cx);
        let cell = out.root();
        assert!(cx.find_var_loc_in_frame(f, v, out).unwrap());
        assert_eq!(cell.bind(cx).sexptype(), LISTSXP);
        let c = cell.bind(cx).as_cons().unwrap();
        assert_eq!(real(c.car()), 2.0);
        assert_eq!(c.tag(), v.bind(cx));
        assign(cx, "v", 3.0, f).unwrap();
        assert_eq!(real(cell.bind(cx).as_cons().unwrap().car()), 3.0, "a write through the frame reaches the cell");
        let four = num(cx, s, 4.0);
        cx.set_var_loc_value(cell, four).unwrap();
        assert_eq!(real(get_in(cx, s, "v", f).bind(cx)), 4.0, "a write through the cell reaches the frame");
        let out = s.out(cx);
        let again = out.root();
        cx.find_var_loc_in_frame(f, v, out).unwrap();
        assert_eq!(again.bind(cx), cell.bind(cx), "the same cell");
        let out = s.out(cx);
        let rv = out.root();
        cx.var_loc_value(cell, out).unwrap();
        assert_eq!(real(rv.bind(cx)), 4.0);
        assert_eq!(ls(cx, f, false), "v a");
        // an unboxed binding is boxed into its cell
        let out = s.out(cx);
        let ca = out.root();
        cx.find_var_loc_in_frame(f, a, out).unwrap();
        assert_eq!(real(ca.bind(cx).as_cons().unwrap().car()), 1.0);
        // a lock reaches the cell
        cx.lock_binding(v, f).unwrap();
        assert!(cx.binding_is_locked(v, f).unwrap());
        let r = cx.set_var_loc_value(cell, four);
        assert_eq!(msg(cx, r), "cannot change value of locked binding for 'v'");
        cx.unlock_binding(v, f).unwrap();
        // rm detaches the cell
        assert!(rm(cx, "v", f));
        let c = cell.bind(cx).as_cons().unwrap();
        assert_eq!(c.car(), cx.unbound());
        assert!(!cx.exists_var_in_frame(f, v).unwrap());
        assign(cx, "v", 5.0, f).unwrap();
        assert_eq!(cell.bind(cx).as_cons().unwrap().car(), cx.unbound(), "a new binding, not the old cell");
        assert_eq!(ls(cx, f, false), "v a");
    });
}

// ---- FRAME -----------------------------------------------------------------

/// `FRAME()` of a slot frame is a GNU R-ordered pairlist snapshot (its
/// cells carry the bindings' bits); `SET_FRAME` on it is an error; a hashed
/// frame's FRAME is R_NilValue, as GNU R's.
#[test]
fn frame_is_a_snapshot_and_set_frame_an_error() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "b", "z"]);
        let one = num(cx, s, 1.0);
        let two = num(cx, s, 2.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one), SlotInit::Default(two)]);
        let z = sym(cx, s, "z");
        cx.define_scalar(z, Scalar::Int(9), f).unwrap();
        let b = sym(cx, s, "b");
        cx.lock_binding(b, f).unwrap();
        let out = s.out(cx);
        let fr = out.root();
        cx.frame_snapshot(f, out).unwrap();
        let mut names = Vec::new();
        let mut x = fr.bind(cx);
        while let Some(c) = x.as_cons() {
            names.push(String::from_utf8(c.tag().as_sym().unwrap().name().bytes().to_vec()).unwrap());
            if names.last().unwrap() == "b" {
                assert_eq!(c.car().sexptype(), PROMSXP, "the default's promise");
                let gp = unsafe { c.as_ref().sexp().gp() };
                assert_eq!(gp & 15, 2, "MISSING");
                assert_ne!(gp & (1 << 14), 0, "the lock");
            }
            x = c.cdr();
        }
        assert_eq!(names, ["z", "a", "b"]);
        // the snapshot does not write back
        let c = fr.bind(cx).as_cons().unwrap();
        unsafe { c.as_ref().sexp().set_car(one.bind(cx).sexp()) };
        assert_eq!(get_in(cx, s, "z", f).bind(cx).as_vector::<i32>().unwrap().get(0), 9);
        let r = cx.set_frame(f, fr);
        assert!(msg(cx, r).contains("SET_FRAME"));
        let g = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let gf = out.root();
        cx.frame_snapshot(g, out).unwrap();
        assert_eq!(gf.bind(cx), cx.nil());
    });
}

// ---- `<<-` -------------------------------------------------------------------

/// `<<-`'s memo: from a frame, through a frame whose shape lacks the
/// symbol, to the shape slot that binds it. A second frame of the same
/// shapes hits the memo; an overflow binding on the way, or a different
/// shape, misses it and `setVar`'s walk decides.
#[test]
fn super_assignment_memo_hits_and_misses() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let outer_shape = shape(cx, s, &["bal"]);
        let mid_shape = shape(cx, s, &["y"]);
        let inner_shape = shape(cx, s, &["x"]);
        let mk = |cx: &mut Cx, sh: Root<'_>, enclos: Root<'_>| {
            let out = s.out(cx);
            let r = out.root();
            cx.new_closure_frame(sh, enclos, g, &[], out).unwrap();
            r
        };
        let outer = mk(cx, outer_shape, g);
        assign(cx, "bal", 0.0, outer).unwrap();
        let mid = mk(cx, mid_shape, outer);
        let inner = mk(cx, inner_shape, mid);
        let bal = sym(cx, s, "bal");
        let mut cache = SuperAssignCache::default();
        for i in 1..=3 {
            let v = num(cx, s, i as f64);
            // `bal <<- i` in a frame enclosed by `inner`
            let here = mk(cx, inner_shape, inner);
            cx.set_var_super(bal, v, here, &mut cache).unwrap();
            assert!(cache.is_filled(), "the path is remembered");
            assert_eq!(real(get_in(cx, s, "bal", outer).bind(cx)), i as f64);
        }
        // another chain of the same shapes hits it
        let outer2 = mk(cx, outer_shape, g);
        assign(cx, "bal", 0.0, outer2).unwrap();
        let mid2 = mk(cx, mid_shape, outer2);
        let inner2 = mk(cx, inner_shape, mid2);
        let here2 = mk(cx, inner_shape, inner2);
        let v = num(cx, s, 10.0);
        cx.set_var_super(bal, v, here2, &mut cache).unwrap();
        assert_eq!(real(get_in(cx, s, "bal", outer2).bind(cx)), 10.0);
        assert_eq!(real(get_in(cx, s, "bal", outer).bind(cx)), 3.0, "the other chain is untouched");
        // an overflow binding on the way takes it
        assign(cx, "bal", -1.0, mid2).unwrap();
        let v = num(cx, s, 11.0);
        cx.set_var_super(bal, v, here2, &mut cache).unwrap();
        assert_eq!(real(get_in(cx, s, "bal", mid2).bind(cx)), 11.0);
        assert_eq!(real(get_in(cx, s, "bal", outer2).bind(cx)), 10.0);
        // no binding anywhere: defined in the global environment
        let nobody = sym(cx, s, "nobody_q");
        let mut c2 = SuperAssignCache::default();
        let v = num(cx, s, 1.0);
        cx.set_var_super(nobody, v, here2, &mut c2).unwrap();
        assert!(!c2.is_filled());
        assert_eq!(real(get_in(cx, s, "nobody_q", g).bind(cx)), 1.0);
    });
}

// ---- the rest of envir.c on slot frames -------------------------------------

/// Locks, active bindings, versions and the special-symbol skip, on slot
/// frames.
#[test]
fn locks_active_bindings_and_versions_on_slot_frames() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "fun"]);
        let one = num(cx, s, 1.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one)]);
        let a = sym(cx, s, "a");
        cx.lock_binding(a, f).unwrap();
        let r = assign(cx, "a", 2.0, f);
        assert_eq!(msg(cx, r), "cannot change value of locked binding for 'a'");
        cx.lock_environment(f, false).unwrap();
        let r = assign(cx, "brand_new", 2.0, f);
        assert_eq!(msg(cx, r), "cannot add bindings to a locked environment");
        // active, in a shape slot and in the overflow block
        let sh = shape(cx, s, &["act"]);
        let f = call_frame(cx, s, sh, &[]);
        let five = num(cx, s, 5.0);
        let fun = s.root(cx, closure(cx, five.bind(cx)));
        for name in ["act", "act2"] {
            let k = sym(cx, s, name);
            cx.make_active_binding(k, fun, f).unwrap();
            assert!(cx.binding_is_active(k, f).unwrap());
            assert_eq!(real(get_in(cx, s, name, f).bind(cx)), 5.0);
        }
        // versions: a function binding moves the box
        let out = s.out(cx);
        let bx = out.root();
        cx.env_version_box(f, out).unwrap();
        let v0 = cx.box_version(bx.bind(cx));
        assign(cx, "plain", 1.0, f).unwrap();
        assert_eq!(cx.box_version(bx.bind(cx)), v0);
        assign_obj(cx, "fun", fun, f).unwrap();
        assert_eq!(cx.box_version(bx.bind(cx)), v0 + 1);
        // NO_SPECIAL_SYMBOLS: set on a frame without special symbols, and
        // cleared when one is defined
        assert!(cx.env_has_no_special_symbols(f.bind(cx)));
        cx.set_no_special_symbols(f);
        let plus = sym(cx, s, "+");
        let nil = cx.nil();
        let pf = s.root(cx, closure(cx, nil));
        cx.define_var(plus, pf, f).unwrap();
        assert!(!cx.env_has_no_special_symbols(f.bind(cx)));
        let out = s.out(cx);
        let r = out.root();
        cx.find_fun3(plus, f, None, out).unwrap();
        assert_eq!(r.bind(cx), pf.bind(cx), "the frame is searched again");
    });
}

/// Definition stamps survive the counter wrapping: bindings are renumbered
/// in their order.
#[test]
fn stamps_renumber_when_the_counter_wraps() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let sh = shape(cx, s, &["a", "b"]);
        let one = num(cx, s, 1.0);
        let f = call_frame(cx, s, sh, &[SlotInit::Value(one), SlotInit::Value(one)]);
        assign(cx, "c", 1.0, f).unwrap();
        // the frame's next stamp, near the end of its range
        unsafe {
            let m = f.bind(cx).sexp().0.add(robigo_heap::slots::OFF_META) as *mut u64;
            *m = (*m & 0xFFFF_FFFF) | ((u32::MAX as u64 - 1) << 32);
        }
        for n in ["d", "e", "g"] {
            assign(cx, n, 1.0, f).unwrap();
        }
        assert_eq!(ls(cx, f, false), "g e d c a b");
    });
}

// ---- collections ----------------------------------------------------------

/// Everything a slot frame holds survives collections: values (in shape
/// slots and the overflow block, through its growth), delayed arguments'
/// expressions and caller, forced values, active functions, cells, the
/// shape. Under `ROBIGO_GCTORTURE=1` (CI's rt job) freed memory is
/// poisoned, so a lost object reads wrong here.
#[cfg(not(miri))]
#[test]
fn slot_frames_keep_their_objects_alive() {
    let mut iso = iso();
    let cx = iso.cx();
    cx.set_eval(test_eval);
    cx.scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let caller = s.root(cx, cx.new_env(g.bind(cx), false, 0).unwrap());
        assign(cx, "in_caller", 77.0, caller).unwrap();
        let names: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let mut all: Vec<&str> = vec!["d", "fo", "act", "cel"];
        all.extend(names.iter().map(|n| n.as_str()));
        let sh = shape(cx, s, &all);
        let (e1, e2) = (num(cx, s, 11.0), num(cx, s, 12.0));
        let out = s.out(cx);
        let f = out.root();
        cx.new_closure_frame(
            sh,
            g,
            caller,
            &[SlotInit::Delayed { expr: e1, value: None }, SlotInit::Forced { expr: e1, value: e2 }],
            out,
        )
        .unwrap();
        let body = num(cx, s, 33.0);
        let fun = s.root(cx, closure(cx, body.bind(cx)));
        let act = sym(cx, s, "act");
        cx.make_active_binding(act, fun, f).unwrap();
        assign(cx, "cel", 44.0, f).unwrap();
        let cel = sym(cx, s, "cel");
        let out = s.out(cx);
        let cell = out.root();
        cx.find_var_loc_in_frame(f, cel, out).unwrap();
        for i in 0..40 {
            // half in shape slots, half in the overflow block
            let name = if i < 20 { names[i].clone() } else { format!("o{i}") };
            assign(cx, &name, i as f64, f).unwrap();
            if i % 7 == 0 {
                collect(cx);
                for _ in 0..300 {
                    let _ = cx.alloc_copy(&[-1.0; 8]);
                }
            }
        }
        // drop every other root to the objects: only the frame keeps them
        collect(cx);
        for _ in 0..3000 {
            let _ = cx.alloc_copy(&[-1.0; 8]);
        }
        for i in 0..40 {
            let name = if i < 20 { names[i].clone() } else { format!("o{i}") };
            assert_eq!(real(get_in(cx, s, &name, f).bind(cx)), i as f64, "{name}");
        }
        assert_eq!(real(get_in(cx, s, "act", f).bind(cx)), 33.0);
        assert_eq!(real(cell.bind(cx).as_cons().unwrap().car()), 44.0);
        let d = get_in(cx, s, "d", f);
        let View::Promise(p) = d.bind(cx).view() else { panic!() };
        assert_eq!(real(p.code()), 11.0);
        let ce = s.root(cx, p.env());
        assert_eq!(real(get_in(cx, s, "in_caller", ce).bind(cx)), 77.0);
        let fo = get_in(cx, s, "fo", f);
        let View::Promise(p) = fo.bind(cx).view() else { panic!() };
        assert_eq!(real(p.value()), 12.0);
    });
}
