//! Cost probes for contexts (ignored; run under callgrind, N against 2N,
//! as `docs/ledger/g3/2026-11-13-c2-contexts.md` records):
//!
//! ```text
//! COST_N=100000 valgrind --tool=callgrind <test binary> --ignored --exact <probe>
//! ```
//!
//! Each probe runs `COST_N` iterations of one shape; `baseline` is the same
//! work without a record, so a probe's difference to it is the record's
//! cost per iteration.

#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_rtcore::{Cx, FunctionContext, Options, OwnedIsolate, Promargs, RResult, Root, ctxt};
use std::hint::black_box;

fn n() -> usize {
    std::env::var("COST_N").ok().and_then(|v| v.parse().ok()).unwrap_or(1000)
}

fn no_on_exit(_: &mut Cx, _: Root<'_>, _: Root<'_>) -> RResult<()> {
    unreachable!()
}

fn with_isolate(f: impl FnOnce(&mut Cx, usize)) {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 12, ..Options::default() });
    f(iso.cx(), n());
}

/// The body alone: a scope with a result slot, written.
#[test]
#[ignore]
fn baseline() {
    with_isolate(|cx, n| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            for _ in 0..n {
                cx.scope(|cx, s| {
                    let out = s.out(cx);
                    out.set(cx, nil.bind(cx));
                });
                black_box(&*cx);
            }
        })
    });
}

/// A closure call's record with no on.exit code (`R_execClosure`'s
/// `begincontext`/`endcontext`), around the same body.
#[test]
#[ignore]
fn function_record() {
    with_isolate(|cx, n| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let spec = FunctionContext {
                callflag: ctxt::RETURN,
                call: nil,
                cloenv: nil,
                sysparent: nil,
                callfun: nil,
                promargs: Promargs::Filled(nil),
            };
            for _ in 0..n {
                cx.scope(|cx, s| {
                    let out = s.out(cx);
                    cx.function_context(&spec, out, &mut no_on_exit, |cx, _, out| {
                        out.set(cx, nil.bind(cx));
                        Ok(())
                    })
                    .unwrap();
                });
                black_box(&*cx);
            }
        })
    });
}

/// A loop record (`with_context`), around the same body.
#[test]
#[ignore]
fn plain_record() {
    with_isolate(|cx, n| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            for _ in 0..n {
                cx.scope(|cx, s| {
                    let out = s.out(cx);
                    let _ = cx.with_context(ctxt::LOOP, nil, nil, nil, None, |cx, _| {
                        out.set(cx, nil.bind(cx));
                        Ok(())
                    });
                });
                black_box(&*cx);
            }
        })
    });
}

#[cfg(not(miri))]
unsafe extern "C" fn nothing(_: usize) -> usize {
    0
}

/// A six-argument native entry of a C function that does nothing (an
/// ALTREP method's shape): record, suspension, trampoline, segment.
#[cfg(not(miri))]
#[test]
#[ignore]
fn native6_entry() {
    with_isolate(|cx, n| {
        for _ in 0..n {
            let r = unsafe { cx.call_native6(nothing as *const std::ffi::c_void, [0; 6], robigo_rtcore::Ret6::Word) };
            black_box(r.unwrap());
        }
    });
}

/// [`function_record`] through the raw layer (the VM's way: its objects
/// are in its own slots already, no root is bound).
#[test]
#[ignore]
fn function_record_raw() {
    with_isolate(|cx, n| {
        cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let x = nil.bind(cx).sexp();
            let spec = robigo_rtcore::RawFunctionContext {
                callflag: ctxt::RETURN,
                call: x,
                cloenv: x,
                sysparent: x,
                callfun: x,
                promargs: Ok(x),
            };
            for _ in 0..n {
                cx.scope(|cx, s| {
                    let out = s.out(cx);
                    unsafe {
                        cx.function_context_raw(&spec, out, &mut no_on_exit, |cx, _, out| {
                            out.set(cx, nil.bind(cx));
                            Ok(())
                        })
                    }
                    .unwrap();
                });
                black_box(&*cx);
            }
        })
    });
}
