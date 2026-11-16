//! The builtin-author API of the E6 reviews: R1c's (`build_with`'s
//! outcome, `Scope::capture`, `Root::replace_from` and `is_written`, the
//! standard-vector admission, `R_Visible`) and R1d's (`Root::replace_with`,
//! `Ref::as_scalar`, `build_scalar`, `lang1`..`lang3`, `mk_string`, the
//! pre-installed symbols, `mk_char`'s embedded NUL).

use robigo_rtcore::types::*;
use robigo_rtcore::{Built, Char, Cx, Diag, Encoding, ExitKind, Lgl, OwnedIsolate, RError, RSym, Ref, View};
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

/// A walk through one root pushes no value-stack slot per step, and the
/// slot keeps its object until the closure succeeds (it may read it).
#[test]
fn replace_with_walks_in_place() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // a chain of 100 environments below the global one
        let e = s.root(cx, cx.global_env());
        for _ in 0..100 {
            e.replace_with(cx, |cx| cx.new_env(e.bind(cx), false, 0)).unwrap();
        }
        let before = cx.stack_depth();
        let mut depth = 0;
        while e.bind(cx) != cx.global_env() {
            e.replace_with(cx, |cx| match e.bind(cx).view() {
                View::Env(r) => Ok(r.enclos()),
                _ => unreachable!(),
            })
            .unwrap();
            depth += 1;
        }
        assert_eq!(depth, 100);
        assert_eq!(cx.stack_depth(), before);
    });
}

/// A length-1 vector's element of the asked type; None for another type,
/// length or an ALTREP vector.
#[test]
fn as_scalar_reads_one_element() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let d = s.root(cx, cx.alloc_copy(&[2.5]).unwrap().finish());
        let i = s.root(cx, cx.alloc_copy(&[7]).unwrap().finish());
        let two = s.root(cx, cx.alloc_copy(&[1.0, 2.0]).unwrap().finish());
        let st = s.root(cx, cx.mk_string(b"ab", Encoding::Native).unwrap());
        assert_eq!(d.bind(cx).as_scalar::<f64>(), Some(2.5));
        assert_eq!(i.bind(cx).as_scalar::<i32>(), Some(7));
        assert_eq!(d.bind(cx).as_scalar::<i32>(), None);
        assert_eq!(two.bind(cx).as_scalar::<f64>(), None);
        assert_eq!(st.bind(cx).as_scalar::<Char<'_>>().map(|c| c.bytes().to_vec()), Some(b"ab".to_vec()));
        assert_eq!(st.bind(cx).as_scalar::<Lgl>(), None);
    });
}

/// `build_scalar`, `lang1`..`lang3` (arguments in a pairlist, as GNU R's
/// `lang2` makes them) and `mk_string`.
#[test]
fn scalar_results_and_calls() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let r = s.capture(cx, |cx, o| cx.build_scalar(o, Lgl(1))).unwrap();
        assert_eq!(r.bind(cx).sexptype(), LGLSXP);
        assert_eq!(r.bind(cx).as_scalar::<Lgl>(), Some(Lgl(1)));
        let c = s
            .capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let f = cx.sym(RSym::AsCharacter).into();
                    let a = cx.mk_string(b"x", Encoding::Native)?;
                    let inner = cx.lang1(f)?;
                    cx.lang3(f, a, inner)
                })
            })
            .unwrap();
        let c = c.bind(cx);
        assert_eq!(c.sexptype(), LANGSXP);
        let cell = c.as_cons().unwrap();
        assert_eq!(cell.car().as_sym().unwrap().name().bytes(), b"as.character");
        let a1 = cell.cdr().as_cons().unwrap();
        assert_eq!(cell.cdr().sexptype(), LISTSXP);
        assert_eq!(a1.car().as_scalar::<Char<'_>>().unwrap().bytes(), b"x");
        let a2 = a1.cdr().as_cons().unwrap();
        assert_eq!(a2.car().sexptype(), LANGSXP);
        assert_eq!(a2.car().as_cons().unwrap().cdr(), cx.nil());
        assert_eq!(a2.cdr(), cx.nil());
    });
}

/// The pre-installed symbols are the symbols `install` finds, and reading
/// one cannot fail.
#[test]
fn pre_installed_symbols_are_the_installed_ones() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    for (s, name) in [
        (RSym::Names, &b"names"[..]),
        (RSym::Dim, b"dim"),
        (RSym::DimNames, b"dimnames"),
        (RSym::Class, b"class"),
        (RSym::Tsp, b"tsp"),
        (RSym::Levels, b"levels"),
        (RSym::RowNames, b"row.names"),
        (RSym::Comment, b"comment"),
        (RSym::Srcref, b"srcref"),
        (RSym::Dots, b"..."),
        (RSym::AsCharacter, b"as.character"),
    ] {
        assert_eq!(cx.sym(s).name().bytes(), name);
        assert_eq!(s.name(), name);
        assert!(cx.sym(s) == cx.install(name).unwrap(), "{s:?}");
    }
}

/// A CHARSXP with an embedded NUL is GNU R's error, raised where the build
/// that asked for it answers its refusal.
#[test]
fn an_embedded_nul_is_gnu_rs_error() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let e = cx.build(out, |cx| cx.mk_string(b"a\0b", Encoding::Native)).unwrap_err();
        let (msg, _) = error_of(cx, e);
        assert_eq!(msg, "embedded nul in string: 'a\\0b'");
        // a later string without one is made
        let r = s.capture(cx, |cx, o| cx.build(o, |cx| cx.mk_string(b"ab", Encoding::Native))).unwrap();
        assert_eq!(r.bind(cx).as_scalar::<Char<'_>>().unwrap().bytes(), b"ab");
    });
}
