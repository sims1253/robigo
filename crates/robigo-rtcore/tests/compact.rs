//! robigo-rt's own ALTREP classes (`altrep::own`, GNU R's altclasses.c):
//! base's compact integer and real sequences as `R_compact_intrange` makes
//! them, read through the typed views without expanding, expanded once by
//! `materialize` (the class's `Dataptr`), copied to a standard vector by
//! `shallow_duplicate` (`Duplicate`), and their methods' answers.

use robigo_rtcore::altrep::own::{OwnClass, SORTED_DECR, SORTED_INCR};
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, ExitKind, OwnedIsolate, Ref, Root, Scope};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The message an `RError` carries.
fn error_message(cx: &mut Cx, e: robigo_rtcore::RError) -> String {
    assert_eq!(cx.exit_info(&e).kind, ExitKind::Error);
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let m = out.root();
        cx.resolve_exit(e, Some(out));
        String::from_utf8(m.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec()).unwrap()
    })
}

/// `n1:n2` as `:` makes it.
fn range<'s>(cx: &mut Cx, s: Scope<'s>, n1: i64, n2: i64) -> Root<'s> {
    s.capture(cx, |cx, o| cx.compact_intrange(n1, n2, o)).unwrap()
}

#[test]
fn a_compact_sequence_is_read_without_expanding() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = range(cx, s, 1, 10);
        let r = x.bind(cx);
        assert_eq!(r.own_class(), Some(OwnClass::CompactIntseq));
        assert_eq!((r.sexptype(), r.len()), (INTSXP, Some(10)));
        assert!(!r.own_is_expanded());
        let v = r.as_vector::<i32>().unwrap();
        assert_eq!((v.len(), v.get(0), v.get(9)), (10, 1, 10));
        assert_eq!(&*v.elements(), &(1..=10).collect::<Vec<_>>()[..]);
        assert_eq!(v.dataptr_or_null(), None);
        assert_eq!(r.compact_info(), (10, 1.0, 1.0));
        assert_eq!((r.own_is_sorted(), r.own_no_na(), r.own_sum()), (Some(SORTED_INCR), Some(true), Some(Ok(55))));
        assert!(!r.maybe_shared() || r.refcnt() > 1, "marked not mutable");
        assert!(!x.bind(cx).own_is_expanded(), "the reads expanded nothing");

        let d = range(cx, s, 5, -2);
        let dv = d.bind(cx).as_vector::<i32>().unwrap();
        assert_eq!(&*dv.elements(), &[5, 4, 3, 2, 1, 0, -1, -2][..]);
        assert_eq!(d.bind(cx).own_is_sorted(), Some(SORTED_DECR));
        assert_eq!(d.bind(cx).own_sum(), Some(Ok(12)));

        // a range past the integer range is a compact real sequence; a
        // length-1 range is the standard scalar
        let big = range(cx, s, 2147483647, 2147483650);
        assert_eq!(big.bind(cx).own_class(), Some(OwnClass::CompactRealseq));
        assert_eq!(big.bind(cx).as_vector::<f64>().unwrap().get(3), 2147483650.0);
        assert_eq!(big.bind(cx).own_sum(), Some(Err(8589934594.0)));
        let one = range(cx, s, 7, 7);
        assert_eq!((one.bind(cx).is_altrep(), one.bind(cx).as_ints()), (false, Some(&[7][..])));
    });
}

#[test]
fn a_slice_of_an_unexpanded_sequence_panics() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = range(cx, s, 1, 3);
        let r = x.bind(cx);
        let p = catch_unwind(AssertUnwindSafe(|| r.as_ints().map(|s| s.len()))).unwrap_err();
        let msg = p.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("was read as a slice: materialize it first"), "{msg}");
    });
}

/// In a debug build the panic names the site that read the slice, not the
/// accessor (the skew between #802 and #803 panicked deep in `as_slice`).
#[cfg(debug_assertions)]
#[test]
fn a_debug_build_names_the_site_that_read_the_slice() {
    use std::cell::RefCell;
    thread_local!(static AT: RefCell<Option<(String, u32)>> = const { RefCell::new(None) });
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        AT.with(|a| *a.borrow_mut() = info.location().map(|l| (l.file().to_string(), l.line())));
        prev(info)
    }));
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = range(cx, s, 1, 3);
        let r = x.bind(cx);
        let line = line!() + 1;
        let p = catch_unwind(AssertUnwindSafe(|| r.as_ints().map(|s| s.len())));
        assert!(p.is_err());
        let (file, at) = AT.with(|a| a.borrow().clone()).expect("a panic location");
        assert!(file.ends_with("compact.rs") && at == line, "the panic was located at {file}:{at}");
    });
}

#[test]
fn materialize_expands_once_and_duplicate_makes_a_standard_vector() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = range(cx, s, 3, 6);
        cx.materialize(x).unwrap();
        assert!(x.bind(cx).own_is_expanded());
        let e = x.bind(cx).sexp();
        assert_eq!(x.bind(cx).as_ints(), Some(&[3, 4, 5, 6][..]));
        // again: the same expansion
        cx.materialize(x).unwrap();
        assert_eq!(x.bind(cx).sexp(), e);
        assert_eq!(x.bind(cx).as_vector::<i32>().unwrap().get(2), 5);
        // a standard vector is left alone
        let plain = s.root(cx, cx.alloc_copy(&[1i32, 2]).unwrap().finish());
        cx.materialize(plain).unwrap();
        assert!(!plain.bind(cx).is_altrep());

        let y = range(cx, s, 10, 8);
        let d = s.capture(cx, |cx, o| cx.shallow_duplicate(y, o)).unwrap();
        let dr = d.bind(cx);
        assert!(!dr.is_altrep());
        assert_eq!(dr.as_ints(), Some(&[10, 9, 8][..]));
        assert!(!y.bind(cx).own_is_expanded(), "Duplicate reads by region");
        let dd = s.capture(cx, |cx, o| cx.duplicate(y, o)).unwrap();
        assert_eq!(dd.bind(cx).as_ints(), Some(&[10, 9, 8][..]));
    });
}

#[test]
fn materialize_nested_expands_the_elements_of_a_list() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let a = range(cx, s, 1, 4);
        let b = range(cx, s, 2, 5);
        let inner = {
            let mut l = cx.alloc_vector::<Ref<'_>>(1).unwrap();
            l.set(0, b.bind(cx));
            s.root(cx, l.finish())
        };
        let outer = {
            let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
            l.set(0, a.bind(cx));
            l.set(1, inner.bind(cx));
            s.root(cx, l.finish())
        };
        // one level: the direct elements only
        cx.materialize_nested(outer, Some(1)).unwrap();
        assert!(a.bind(cx).own_is_expanded());
        assert!(!b.bind(cx).own_is_expanded());
        cx.materialize_nested(outer, None).unwrap();
        assert!(b.bind(cx).own_is_expanded());
    });
}

#[test]
fn unserialize_refuses_an_increment_other_than_one() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let e = cx.scope(|cx, s| {
        s.capture(cx, |cx, o| cx.compact_unserialize(OwnClass::CompactIntseq, 3, 1.0, 2.0, o)).unwrap_err()
    });
    assert_eq!(error_message(cx, e), "compact sequences with increment 2 not supported yet");
    cx.scope(|cx, s| {
        let x = s.capture(cx, |cx, o| cx.compact_unserialize(OwnClass::CompactRealseq, 3, 0.5, -1.0, o)).unwrap();
        assert_eq!(&*x.bind(cx).as_vector::<f64>().unwrap().elements(), &[0.5, -0.5, -1.5][..]);
    });
}
