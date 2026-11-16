//! E6 (docs/g3.md §10, §15.3): the instruction cost of reaching the
//! evaluator through the isolate's hook table, with robigo-rt's own hooks
//! installed. B1 measured the table with stand-in hooks before an
//! evaluator existed (robigo-rtbuiltins' tests/hook_cost.rs); these are
//! the same two shapes over the real ones:
//! - `lapply-hook` / `lapply-direct`: B2b's `lapply` shape over 16
//!   elements, a real closure `function(x) x` called per element through
//!   `cx.apply` (the table, as `do_lapply` calls it) against
//!   `robigo_rt::apply_hook` called by name;
//! - `dispatch-hook` / `dispatch-direct`: `[`'s `DispatchOrEval` on an
//!   object whose class has no method, through `cx.try_dispatch` against
//!   its pre-test and `robigo_rt::dispatch::dispatch_hook` called by name;
//! - `dispatch-plain`: `cx.try_dispatch` on a non-object, the pre-test
//!   alone.
//!
//! The callee's work is the same in a mode and its direct twin, so their
//! difference is the table's own cost. Ignored: run one mode N times
//! under callgrind, N against 2N:
//!
//! ```text
//! ROBIGO_HOOK_COST=lapply-hook:10000 valgrind --tool=callgrind \
//!     <the hook_cost test binary> --ignored --exact hook_cost
//! ```

mod common;

use common::*;
use robigo_rt::Profile;
use robigo_rtcore::{Cx, Dispatched, Generic, Ref, Root, Scope};
use std::hint::black_box;

const SUBSET: Generic = Generic::builtin("[");

/// The value of `src` (one expression) evaluated in the global
/// environment, rooted in `s`.
fn value<'s>(cx: &mut Cx, s: Scope<'s>, src: &str) -> Root<'s> {
    let ex = s.out(cx);
    let exr = ex.root();
    robigo_rt::lower::parse(cx, src, ex).unwrap_or_else(|_| panic!("parse {src}"));
    let e = s.root(cx, exr.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0));
    let g = s.root(cx, cx.global_env());
    let o = s.out(cx);
    let r = o.root();
    robigo_rt::eval(cx, e, g, o).unwrap_or_else(|_| panic!("eval {src}"));
    r
}

/// B2b's `lapply` shape over the list `x`: one call of `f` per element,
/// each result rooted until the list is built.
#[inline(never)]
fn lapply(cx: &mut Cx, x: Root<'_>, f: Root<'_>, call: Root<'_>, env: Root<'_>, direct: bool) -> usize {
    cx.scope(|cx, s| {
        let n = x.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
        let mut res = Vec::with_capacity(n);
        for i in 0..n {
            let xi = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i);
            let args = s.args(cx, &[xi], None);
            let o = s.out(cx);
            res.push(o.root());
            if direct {
                robigo_rt::apply_hook(cx, call, f, args, env, o).unwrap();
            } else {
                cx.apply(call, f, args, env, o).unwrap();
            }
        }
        res.iter().map(|r| r.bind(cx).sexptype() as usize).sum()
    })
}

#[test]
#[ignore = "a measurement: run under callgrind with ROBIGO_HOOK_COST=mode:N"]
fn hook_cost() {
    let spec = std::env::var("ROBIGO_HOOK_COST").unwrap_or_else(|_| "lapply-hook:100".into());
    let (mode, n) = spec.split_once(':').unwrap();
    let n: usize = n.parse().unwrap();
    // the mode as a number, chosen once: a string match per iteration would
    // cost each mode differently
    const MODES: [&str; 5] = ["lapply-hook", "lapply-direct", "dispatch-hook", "dispatch-plain", "dispatch-direct"];
    let m = black_box(MODES.iter().position(|&x| x == mode).unwrap_or_else(|| panic!("unknown mode {mode}")));
    let mut r = Session::new(Profile::Bc);
    r.val("f <- function(x) x; l <- list(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)");
    r.val("o <- 1; class(o) <- \"foo\"; p <- 1");
    let cx = r.cx();
    cx.scope(|cx, s| {
        let f = value(cx, s, "f");
        let l = value(cx, s, "l");
        let obj = value(cx, s, "o");
        let plain = value(cx, s, "p");
        let call = value(cx, s, "quote(FUN(X[[i]], ...))");
        let sub = value(cx, s, "quote(o[1])");
        let env = s.root(cx, cx.global_env());
        let mut acc = 0usize;
        for _ in 0..n {
            acc += cx.scope(|cx, s| match m {
                0 => lapply(cx, l, f, call, env, false),
                1 => lapply(cx, l, f, call, env, true),
                2 | 3 => {
                    let a = if m == 2 { obj } else { plain };
                    let args = s.args(cx, &[a.bind(cx)], None);
                    let out = s.out(cx);
                    let r = out.root();
                    match cx.try_dispatch(&SUBSET, black_box(sub), args, env, out).unwrap() {
                        Dispatched::Done => {}
                        Dispatched::Default(out) => out.set(cx, a.bind(cx)),
                    }
                    r.bind(cx).sexptype() as usize
                }
                _ => {
                    let args = s.args(cx, &[obj.bind(cx)], None);
                    let out = s.out(cx);
                    let r = out.root();
                    // the same pre-test, then the dispatcher by name
                    let out = if args.get(cx, 0).is_object() {
                        match robigo_rt::dispatch::dispatch_hook(cx, &SUBSET, black_box(sub), args, env, out).unwrap() {
                            Dispatched::Done => return r.bind(cx).sexptype() as usize,
                            Dispatched::Default(out) => out,
                        }
                    } else {
                        out
                    };
                    out.set(cx, obj.bind(cx));
                    r.bind(cx).sexptype() as usize
                }
            });
        }
        black_box(acc);
    });
}
