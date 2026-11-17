//! `ROBIGO_RETRY_TORTURE=1` ([`robigo_rtcore::Cx::retry_torture`],
//! docs/g3.md §16, E9): the first allocation of every `build_with` and
//! `retry_alloc` call is refused once, so each one's retry runs; nothing
//! else is refused, and with the mode off nothing is.

use robigo_rtcore::{Cx, OwnedIsolate, Safepoint};
use std::cell::Cell;

/// The collections made for a refused allocation's retry.
fn retries(cx: &Cx) -> u64 {
    cx.gc_stats().collections_by_kind[Safepoint::Retry as usize]
}

// (a refused allocation is retried after a collection, which Miri does
// not run)
#[cfg(not(miri))]
#[test]
fn each_build_and_retry_alloc_runs_its_retry_once() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.set_retry_torture(true);
    cx.scope(|cx, s| {
        let before = retries(cx);
        let tries = Cell::new(0);
        let out = s.out(cx);
        let r = out.root();
        cx.build(out, |cx| {
            tries.set(tries.get() + 1);
            Ok(cx.alloc_copy(&[1.5])?.finish().into())
        })
        .unwrap();
        assert_eq!(tries.get(), 2, "the first attempt is refused, the second builds");
        assert_eq!(retries(cx), before + 1);
        assert_eq!(r.bind(cx).as_vector::<f64>().unwrap().get(0), 1.5);

        let tries = Cell::new(0);
        cx.retry_alloc(|cx| {
            tries.set(tries.get() + 1);
            cx.install(b"retry_torture_fresh_symbol").map(|_| ())
        })
        .unwrap();
        assert_eq!(tries.get(), 2);
        assert_eq!(retries(cx), before + 2);
    });
}

/// A build that allocates nothing runs once and leaves no refusal behind:
/// the next allocation, outside any build, is not refused (no collection,
/// so Miri runs this too). With the mode off, an allocating build runs
/// once.
#[test]
fn nothing_else_is_refused() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.set_retry_torture(true);
    cx.scope(|cx, s| {
        let tries = Cell::new(0);
        let out = s.out(cx);
        cx.build(out, |cx| {
            tries.set(tries.get() + 1);
            Ok(cx.nil())
        })
        .unwrap();
        assert_eq!(tries.get(), 1);
        assert!(cx.alloc_copy(&[2.0]).is_ok(), "an allocation after the build was refused");

        cx.set_retry_torture(false);
        let tries = Cell::new(0);
        let out = s.out(cx);
        cx.build(out, |cx| {
            tries.set(tries.get() + 1);
            Ok(cx.alloc_copy(&[3.0])?.finish().into())
        })
        .unwrap();
        assert_eq!(tries.get(), 1);
        assert_eq!(retries(cx), 0);
    });
}
