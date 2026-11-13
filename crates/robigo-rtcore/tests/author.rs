//! The builtin-author API of the E6 review (R1c): `build_with`'s outcome,
//! `Scope::capture`, `Root::replace_from` and `is_written`, the
//! standard-vector admission, `R_Visible`.

use robigo_rtcore::types::*;
use robigo_rtcore::{Built, Char, Cx, Diag, ExitKind, OwnedIsolate, RError, Ref};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The message and call of the pending error, resolved.
fn error_of(cx: &mut Cx, e: RError) -> (String, bool) {
    assert_eq!(cx.exit_info(&e).kind, ExitKind::Error);
    let has_call = cx.exit_call(&e) != cx.nil();
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let m = out.root();
        cx.resolve_exit(e, Some(out));
        let msg = String::from_utf8(m.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec()).unwrap();
        (msg, has_call)
    })
}

/// The data a successful attempt returns, with no flag outside the closure.
#[test]
fn build_with_returns_the_successful_attempts_data() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_copy(&[1, i32::MAX, 3]).unwrap().finish());
        let out = s.out(cx);
        let res = out.root();
        let overflowed = cx
            .build_with(out, |cx| {
                let xs = x.bind(cx).as_vector::<i32>().unwrap().as_slice().unwrap();
                let mut overflowed = 0;
                let v = cx.alloc_with(xs.len(), |i| {
                    xs[i].checked_add(1).unwrap_or_else(|| {
                        overflowed += 1;
                        i32::MIN
                    })
                })?;
                Ok(Built::done(v.finish()).with(overflowed))
            })
            .unwrap();
        assert_eq!(overflowed, 1);
        assert_eq!(res.bind(cx).as_vector::<i32>().unwrap().as_slice(), Some(&[2, i32::MIN, 4][..]));
    });
}

/// A diagnostic found under `&Cx` is raised after the interval, with the
/// call it names, and nothing is written.
#[test]
fn build_with_raises_the_diagnostic_after_the_interval() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let call = s.root(cx, cx.lcons(cx.install(b"f").unwrap().into(), cx.nil()).unwrap());
        for (with_call, d) in
            [(false, Diag::error("bad element 2")), (true, Diag::error_call(Some(call), "bad element 2"))]
        {
            let out = s.out(cx);
            let res = out.root();
            let e = cx.build_with(out, |_| Ok(Built::<'_, '_, ()>::fail(d.clone()))).unwrap_err();
            assert_eq!(error_of(cx, e), ("bad element 2".to_string(), with_call));
            assert!(!res.is_written(cx));
        }
        assert_eq!(Diag::error("m").message(), "m");
    });
}

#[test]
fn capture_roots_what_the_callee_writes() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let v = s.capture(cx, |cx, out| cx.build(out, |cx| Ok(cx.alloc_copy(&[7.0])?.finish().into()))).unwrap();
        assert_eq!(v.bind(cx).as_vector::<f64>().unwrap().get(0), 7.0);
        // an explicit NULL is a written result
        let n = s.capture(cx, |cx, out| {
            out.set(cx, cx.nil());
            Ok(())
        });
        assert_eq!(n.unwrap().bind(cx), cx.nil());
        // an error passes through, with nothing captured
        let e = s.capture(cx, |cx, out| {
            let _ = out;
            Err(cx.error("stop"))
        });
        let e = e.unwrap_err();
        assert_eq!(error_of(cx, e).0, "stop");
    });
}

#[test]
fn capture_refuses_an_unwritten_result() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let p = catch_unwind(AssertUnwindSafe(|| {
        cx.scope(|cx, s| {
            let _ = s.capture(cx, |_, out| {
                let _ = out;
                Ok(())
            });
        })
    }));
    let p = p.unwrap_err();
    let msg = p.downcast_ref::<&str>().copied().expect("a panic whose payload is a string");
    assert!(msg.contains("without writing its Out"), "{msg}");
}

#[test]
fn replace_from_swaps_a_root_in_one_step() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let current = s.root(cx, cx.alloc_copy(&[1]).unwrap().finish());
        let next = s.root(cx, cx.alloc_copy(&[2]).unwrap().finish());
        let n = next.bind(cx).sexp();
        current.replace_from(cx, next);
        assert_eq!(current.bind(cx).sexp(), n);
        assert!(current.is_written(cx));
        // what the scope roots: both slots name the second vector now
        let mut seen = Vec::new();
        cx.for_each_root(&mut |x| seen.push(x));
        assert_eq!(seen.iter().filter(|&&x| x == n).count(), 2);
    });
}

/// The admission: a total length for NULL and standard vectors, none for a
/// package ALTREP object or a non-vector.
#[test]
fn std_vector_admits_what_has_a_total_length() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let cx: &Cx = cx;
    assert_eq!(cx.nil().std_vector().map(|v| v.len()), Some(0));
    let v = cx.alloc_copy(&[1.0, 2.0]).unwrap().finish().as_ref();
    assert_eq!(v.std_vector().map(|v| (v.len(), v.sexptype())), Some((2, REALSXP)));
    let l = cx.alloc_vector::<Ref<'_>>(3).unwrap().finish().as_ref();
    assert_eq!(l.std_vector().map(|v| v.len()), Some(3));
    let s = cx.install(b"x").unwrap().as_ref();
    assert_eq!(s.std_vector(), None, "a symbol has no vector length");
    assert_eq!(cx.mk_char(b"x", robigo_rtcore::Encoding::Native).unwrap().as_ref().std_vector(), None, "nor a CHARSXP");
    // a package ALTREP vector, in its real shape (a node with nil fields)
    let alt = cx.cons(cx.nil(), cx.nil()).unwrap();
    // SAFETY: fresh and node-sized: flagged ALTREP, then typed INTSXP, it is
    // a well-formed ALTREP vector whose four node words are R_NilValue.
    unsafe {
        alt.sexp().set_flag(robigo_heap::flags::ALTREP, true);
        alt.sexp().set_typ(INTSXP);
    }
    assert_eq!(alt.std_vector(), None, "its length needs &mut Cx");
}

#[test]
fn r_visible_is_the_isolates() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    assert!(cx.visible());
    cx.set_visible(false);
    assert!(!cx.visible());
}
