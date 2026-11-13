//! E6: the instruction cost of reaching the evaluator through the
//! isolate's hook table (docs/g3.md §10) against calling the same function
//! directly. Ignored; run one mode N times under callgrind, as N against 2N
//! (`scripts`-free, see the ledger entry for the command):
//!
//! ```text
//! ROBIGO_HOOK_COST=lapply-hook:100000 valgrind --tool=callgrind \
//!     <the hook_cost test binary> --ignored --exact hook_cost
//! ```
//!
//! Modes (each iteration is one call, or one `lapply` over 16 elements):
//! - `eval-hook` / `eval-direct` / `eval-inline`: `cx.eval(expr, env, out)`
//!   through the table, the evaluator called by name (`#[inline(never)]`),
//!   and inlined (what a call site that knew its evaluator would get).
//! - `lapply-hook` / `lapply-direct`: B2b's `lapply` shape, `cx.apply` per
//!   element against the function called by name.
//! - `dispatch-hook` / `dispatch-direct`: `[`'s `DispatchOrEval` on an
//!   object whose class has no method (the dispatcher answers "default"),
//!   through `cx.try_dispatch` against its pre-test and a direct call.
//! - `dispatch-plain`: `cx.try_dispatch` on a non-object: the pre-test
//!   alone, which makes no indirect call.
//!
//! Until V1 there is no evaluator and no S3 dispatch to run: the hooks here
//! do the least a real one would (read their arguments, write a value), so
//! the difference between a mode and its direct twin is the table's own
//! cost. (A stand-in that ignored an argument would let LLVM drop it from
//! the direct call only, which no real evaluator allows.) V1 re-measures
//! with the VM's `lapply` and `[`.

use robigo_rtcore::{Args, Cx, Dispatched, Generic, Hooks, Options, Out, OwnedIsolate, RResult, Ref, Root};
use std::hint::black_box;

/// The stand-in evaluator: `eval(expr)` is `expr`.
#[inline(never)]
fn eval_id(cx: &mut Cx, expr: Root<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    black_box(env);
    out.set(cx, expr.bind(cx));
    Ok(())
}

#[inline(always)]
fn eval_id_inline(cx: &mut Cx, expr: Root<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    black_box(env);
    out.set(cx, expr.bind(cx));
    Ok(())
}

/// The stand-in `applyClosure`: `f(x)` is `x`.
#[inline(never)]
fn apply_id(cx: &mut Cx, call: Root<'_>, f: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    black_box((call, f, env));
    out.set(cx, args.get(cx, 0));
    Ok(())
}

/// The stand-in dispatcher: no method for any class.
#[inline(never)]
fn dispatch_none<'o>(
    cx: &mut Cx,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    black_box((g, call, args, env, &*cx));
    Ok(Dispatched::Default(out))
}

const SUBSET: Generic = Generic::builtin("[");

/// B2b's `lapply` shape over `x` (a list): one call per element, each
/// result rooted until the list is built.
#[inline(never)]
fn lapply(cx: &mut Cx, x: Root<'_>, f: Root<'_>, env: Root<'_>, direct: bool, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let n = x.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
        let mut res = Vec::with_capacity(n);
        for i in 0..n {
            let xi = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i);
            let args = s.args(cx, &[xi], None);
            let o = s.out(cx);
            res.push(o.root());
            if direct {
                apply_id(cx, f, f, args, env, o)?;
            } else {
                cx.apply(f, f, args, env, o)?;
            }
        }
        cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in res.iter().enumerate() {
                v.set(i, r.bind(cx));
            }
            Ok(v.finish().into())
        })
    })
}

#[test]
#[ignore = "a measurement: run under callgrind with ROBIGO_HOOK_COST=mode:N"]
fn hook_cost() {
    let spec = std::env::var("ROBIGO_HOOK_COST").unwrap_or_else(|_| "eval-hook:1000".into());
    let (mode, n) = spec.split_once(':').unwrap();
    let n: usize = n.parse().unwrap();
    // the mode as a number, chosen once: a string match per iteration would
    // cost each mode differently
    const MODES: [&str; 8] = [
        "eval-hook",
        "eval-direct",
        "eval-inline",
        "lapply-hook",
        "lapply-direct",
        "dispatch-hook",
        "dispatch-plain",
        "dispatch-direct",
    ];
    let m = MODES.iter().position(|&x| x == mode).unwrap_or_else(|| panic!("unknown mode {mode}"));
    let m = black_box(m);
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() });
    let cx = iso.cx();
    cx.install_hooks(Hooks { eval: eval_id, apply: apply_id, dispatch: dispatch_none, ..Hooks::default() });
    cx.scope(|cx, s| {
        let one = cx.alloc_copy(&[1.0]).unwrap().finish();
        let x = s.root(cx, one);
        let env = s.root(cx, cx.global_env());
        // an object: a class attribute
        let o = s.out(cx);
        let obj = o.root();
        cx.build(o, |cx| {
            let k = cx.mk_char(b"foo", robigo_rtcore::Encoding::Native)?;
            let mut kv = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
            kv.set(0, k);
            let attrs = cx.cons_tagged(kv.finish().into(), cx.nil(), cx.install(b"class")?)?;
            let mut v = cx.alloc_copy(&[1.0])?;
            v.set_attrib(attrs);
            v.set_object(true);
            Ok(v.finish().into())
        })
        .unwrap();
        let o = s.out(cx);
        let list = o.root();
        cx.build(o, |cx| {
            let mut l = cx.alloc_vector::<Ref<'_>>(16)?;
            for i in 0..16 {
                l.set(i, x.bind(cx));
            }
            Ok(l.finish().into())
        })
        .unwrap();
        let mut acc = 0usize;
        for _ in 0..n {
            acc += cx.scope(|cx, s| {
                let out = s.out(cx);
                let r = out.root();
                match m {
                    0 => cx.eval(black_box(x), env, out).unwrap(),
                    1 => eval_id(cx, black_box(x), env, out).unwrap(),
                    2 => eval_id_inline(cx, black_box(x), env, out).unwrap(),
                    3 => lapply(cx, list, x, env, false, out).unwrap(),
                    4 => lapply(cx, list, x, env, true, out).unwrap(),
                    5 | 6 => {
                        let a = if m == 5 { obj } else { x };
                        let args = s.args(cx, &[a.bind(cx)], None);
                        match cx.try_dispatch(&SUBSET, black_box(x), args, env, out).unwrap() {
                            Dispatched::Done => {}
                            Dispatched::Default(out) => out.set(cx, a.bind(cx)),
                        }
                    }
                    _ => {
                        let args = s.args(cx, &[obj.bind(cx)], None);
                        // the same pre-test, then the dispatcher by name
                        let out = if args.get(cx, 0).is_object() {
                            match dispatch_none(cx, &SUBSET, black_box(x), args, env, out).unwrap() {
                                Dispatched::Done => return 1,
                                Dispatched::Default(out) => out,
                            }
                        } else {
                            out
                        };
                        out.set(cx, obj.bind(cx));
                    }
                }
                r.bind(cx).sexptype() as usize
            });
        }
        black_box(acc);
    });
}
