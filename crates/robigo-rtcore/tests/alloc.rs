//! R1b: where collections happen (safepoints, `poll_alloc`,
//! `alloc_rooted`) and writes into existing objects (`shallow_duplicate`,
//! `unique_mut`, `unique_list`, `edit_pair`), with GNU R's reference counts
//! (docs/g3.md §2.4, §3 items 2 to 4, §4). The tests that collect are not
//! run under Miri: the collector calls the heap's C shim.

#[cfg(not(miri))]
use robigo_rtcore::Safepoint;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, Encoding, ExitKind, OwnedIsolate, Ref};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The message of the panic `f` raises.
fn panic_message<R>(f: impl FnOnce() -> R) -> String {
    let Err(p) = catch_unwind(AssertUnwindSafe(f)) else { panic!("expected a panic") };
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .expect("a panic whose payload is not a string")
}

/// The message an `RError` carries, and the exit resolved.
fn error_message(cx: &mut Cx, e: robigo_rtcore::RError) -> String {
    assert_eq!(cx.exit_info(&e).kind, ExitKind::Error);
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let m = out.root();
        cx.resolve_exit(e, Some(out));
        String::from_utf8(m.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec()).unwrap()
    })
}

/// Allocate garbage until a collection is due (GNU R's trigger, or at once
/// under `ROBIGO_GCTORTURE=1`).
#[cfg(not(miri))]
fn make_due(cx: &mut Cx) {
    let mut n = 0;
    while !cx.gc_due() {
        let _ = cx.alloc_vector::<f64>(1 << 12).unwrap(); // 32 KiB
        n += 1;
        assert!(n < 100_000, "no collection came due");
    }
}

#[cfg(not(miri))]
fn by_kind(cx: &Cx, k: Safepoint) -> u64 {
    cx.gc_stats().collections_by_kind[k as usize]
}

/// A safepoint serves a due collection with the isolate's roots, and does
/// nothing when none is due.
#[cfg(not(miri))]
#[test]
fn a_safepoint_serves_a_due_collection() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let keep = s.root(cx, cx.alloc_with(100, |i| i as f64).unwrap().finish());
        make_due(cx);
        let before = by_kind(cx, Safepoint::Return);
        cx.safepoint(Safepoint::Return).unwrap();
        assert!(!cx.gc_due());
        assert_eq!(by_kind(cx, Safepoint::Return), before + 1);
        assert_eq!(keep.bind(cx).as_vector::<f64>().unwrap().get(99), 99.0);
        let n = cx.gc_stats().collections;
        cx.safepoint(Safepoint::Loop).unwrap();
        assert_eq!(cx.gc_stats().collections, n, "nothing was due");
    });
}

/// A loop that polls per chunk keeps its garbage within E4's bound.
#[cfg(not(miri))]
#[test]
fn poll_alloc_keeps_a_loops_garbage_within_the_bound() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    for _ in 0..4000 {
        let _ = cx.alloc_vector::<f64>(1 << 12).unwrap(); // 32 KiB a chunk, 125 MiB in all
        cx.poll_alloc(1 << 15).unwrap();
    }
    let st = cx.gc_stats();
    assert!(st.collections_by_kind[Safepoint::PollAlloc as usize] >= 1);
    assert_eq!(st.overshoot_over_bound, 0, "max overshoot {} bytes", st.max_overshoot);
}

/// `alloc_rooted` collects before a large request, as GNU R's
/// `allocVector3` does, so no large `&Cx` request is made while due.
#[cfg(not(miri))]
#[test]
fn alloc_rooted_collects_before_a_large_request() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        make_due(cx);
        let (rooted, large) = (by_kind(cx, Safepoint::Rooted), cx.gc_stats().large_while_due);
        let out = s.out(cx);
        let res = out.root();
        let mut v = cx.alloc_rooted::<f64>(1 << 18).unwrap(); // 2 MiB
        let fill_cx = v.cx();
        v.as_mut_slice()[7] = 7.0;
        out.set(fill_cx, v.finish());
        assert_eq!(by_kind(cx, Safepoint::Rooted), rooted + 1);
        assert_eq!(cx.gc_stats().large_while_due, large);
        let r = res.bind(cx).as_vector::<f64>().unwrap();
        assert_eq!((r.len(), r.get(7), r.get(8)), (1 << 18, 7.0, 0.0));
    });
}

/// The same request under `&Cx` while due is counted. Under
/// `ROBIGO_GC_STATS=check` (the gctorture pass of `scripts/rt-ci.py`) it
/// panics where it is made instead, which is that check's point, so this
/// test skips there and says so in the log: the release pass counts it.
#[cfg(not(miri))]
#[test]
fn a_large_cx_request_while_due_is_counted() {
    if std::env::var("ROBIGO_GC_STATS").as_deref() == Ok("check") {
        // to the stream itself: libtest captures `eprintln!` of a passing test
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "a_large_cx_request_while_due_is_counted: skipped, under ROBIGO_GC_STATS=check a large &Cx request while due panics"
        );
        return;
    }
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    make_due(cx);
    let before = cx.gc_stats().large_while_due;
    let _ = cx.alloc_vector::<f64>(1 << 18).unwrap();
    assert_eq!(cx.gc_stats().large_while_due, before + 1);
}

/// `alloc_rooted` refuses what GNU R refuses, with its message.
#[test]
fn alloc_rooted_refuses_with_gnu_rs_message() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let e = cx.alloc_rooted::<f64>(usize::MAX / 4).map(|_| ()).unwrap_err();
    let m = error_message(cx, e);
    assert!(m.starts_with("cannot allocate vector of size ") && m.ends_with(" Gb"), "{m}");
}

// ---- duplicates and in-place writes (no collection: Miri runs these) ----

/// A list `[a, a]` with a `names` attribute (made through the raw layer:
/// the attribute API is B2's), OBJECT set and a true length.
fn named_list<'g>(cx: &'g Cx, a: Ref<'g>) -> Ref<'g> {
    let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
    l.set(0, a);
    l.set(1, a);
    let l = l.finish().as_ref();
    let mut n = cx.alloc_vector::<Char<'_>>(2).unwrap();
    n.set(0, cx.mk_char(b"x", Encoding::Native).unwrap());
    n.set(1, cx.mk_char(b"y", Encoding::Native).unwrap());
    let cell = cx.cons(n.finish().into(), cx.nil()).unwrap();
    // SAFETY: `cell` and `l` are fresh objects of this heap, unpublished;
    // the stores keep the counts.
    unsafe {
        cell.sexp().set_tag(cx.install(b"names").unwrap().as_ref().sexp());
        l.sexp().set_attrib(cell.sexp());
        l.sexp().set_object(true);
        l.sexp().set_truelen(5);
    }
    l
}

#[test]
fn a_shallow_duplicate_copies_the_spine_and_counts_the_elements() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let a = cx.alloc_copy(&[1.0]).unwrap().finish().as_ref();
        let x = s.root(cx, named_list(cx, a));
        let a = s.root(cx, a);
        assert_eq!(a.bind(cx).refcnt(), 2);
        let out = s.out(cx);
        let d = out.root();
        cx.shallow_duplicate(x, out).unwrap();
        let (xr, dr) = (x.bind(cx), d.bind(cx));
        assert_ne!(xr, dr);
        let (xl, dl) = (xr.as_vector::<Ref<'_>>().unwrap(), dr.as_vector::<Ref<'_>>().unwrap());
        assert_eq!(dl.as_slice(), xl.as_slice(), "the same elements");
        assert_eq!(a.bind(cx).refcnt(), 4, "each gained a reference");
        assert!(dr.is_object());
        // SAFETY: a live vector.
        assert_eq!(unsafe { dr.sexp().truelen() }, 5);
        let (xa, da) = (xr.attrib().unwrap().as_cons().unwrap(), dr.attrib().unwrap().as_cons().unwrap());
        assert_ne!(xa, da, "a new attribute cell");
        assert_eq!((da.car(), da.tag(), da.cdr()), (xa.car(), xa.tag(), cx.nil()));
    });
}

#[test]
fn a_shallow_duplicate_of_a_call_keeps_cars_tags_and_type() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let arg = cx.alloc_copy(&[2]).unwrap().finish().as_ref();
        let tail = cx.cons(arg, cx.nil()).unwrap();
        // SAFETY: a fresh cell.
        unsafe { tail.sexp().set_tag(cx.install(b"n").unwrap().as_ref().sexp()) };
        let call = s.root(cx, cx.lcons(cx.install(b"f").unwrap().into(), tail).unwrap());
        let out = s.out(cx);
        let d = out.root();
        cx.shallow_duplicate(call, out).unwrap();
        let (c, dc) = (call.bind(cx).as_cons().unwrap(), d.bind(cx).as_cons().unwrap());
        assert_ne!(c, dc);
        assert_eq!(d.bind(cx).sexptype(), LANGSXP);
        assert_eq!(dc.car(), c.car());
        let (t, dt) = (c.cdr().as_cons().unwrap(), dc.cdr().as_cons().unwrap());
        assert_ne!(t, dt);
        assert_eq!(dt.as_ref().sexptype(), LISTSXP);
        assert_eq!((dt.car(), dt.tag(), dt.cdr()), (t.car(), t.tag(), cx.nil()));
        // the types GNU R does not copy come back as they are
        let sym = s.root(cx, cx.install(b"f").unwrap());
        let out = s.out(cx);
        let same = out.root();
        cx.shallow_duplicate(sym, out).unwrap();
        assert_eq!(same.bind(cx), sym.bind(cx));
    });
}

#[test]
fn unique_mut_writes_in_place_unless_shared() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // unshared: the same object, written in place
        let x = s.root(cx, cx.alloc_copy(&[1.0, 2.0]).unwrap().finish());
        let addr = x.bind(cx).sexp();
        cx.unique_mut::<f64>(x).unwrap().as_mut_slice()[0] = 10.0;
        assert_eq!(x.bind(cx).sexp(), addr);
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[10.0, 2.0][..]));
        // shared (two list slots): the root gets a duplicate, the list's
        // object is unchanged
        let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
        l.set(0, x.bind(cx));
        l.set(1, x.bind(cx));
        let l = s.root(cx, l.finish());
        assert!(x.bind(cx).maybe_shared());
        cx.unique_mut::<f64>(x).unwrap().as_mut_slice()[1] = 20.0;
        assert_ne!(x.bind(cx).sexp(), addr);
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[10.0, 20.0][..]));
        let old = l.bind(cx).as_vector::<Ref<'_>>().unwrap().get(0);
        assert_eq!(old.sexp(), addr);
        assert_eq!(old.as_vector::<f64>().unwrap().as_slice(), Some(&[10.0, 2.0][..]));
        assert_eq!(x.bind(cx).refcnt(), 0, "the duplicate is referenced by no one");
        // the wrong handle for the type
        assert!(panic_message(|| cx.unique_mut::<i32>(x).map(|_| ())).contains("the wrong handle"));
    });
}

#[test]
fn unique_mut_of_a_traced_shared_vector_is_not_implemented_yet() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_copy(&[1]).unwrap().finish());
        let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
        l.set(0, x.bind(cx));
        l.set(1, x.bind(cx));
        s.root(cx, l.finish());
        // SAFETY: a live object's header; RTRACE is `tracemem`'s bit.
        unsafe {
            let o = x.bind(cx).sexp();
            o.set_extra(o.extra() | 2);
        }
        let e = cx.unique_mut::<i32>(x).map(|_| ()).unwrap_err();
        assert_eq!(error_message(cx, e), "robigo: 'tracemem' is not implemented yet");
    });
}

#[test]
fn unique_list_stores_with_gnu_rs_counts() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let a = s.root(cx, cx.alloc_copy(&[1]).unwrap().finish());
        let b = s.root(cx, cx.alloc_copy(&[2]).unwrap().finish());
        let mut l = cx.alloc_vector::<Ref<'_>>(1).unwrap();
        l.set(0, a.bind(cx));
        let l = s.root(cx, l.finish());
        let b_obj = b.bind(cx).sexp();
        {
            let mut u = cx.unique_list(l).unwrap();
            u.set(0, b);
            assert_eq!(u.get(0).sexp(), b_obj);
        }
        assert_eq!((a.bind(cx).refcnt(), b.bind(cx).refcnt()), (0, 1));
        // a character vector takes CHARSXPs only
        let sv = s.root(cx, cx.alloc_vector::<Char<'_>>(1).unwrap().finish());
        let c = s.root(cx, cx.mk_char(b"z", Encoding::Native).unwrap());
        cx.unique_list(sv).unwrap().set(0, c);
        assert_eq!(sv.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes(), b"z");
        assert!(panic_message(|| cx.unique_list(sv).unwrap().set(0, a)).contains("must be a CHARSXP"));
    });
}

#[test]
fn edit_pair_reads_a_snapshot_when_x_is_y() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        // distinct vectors: x += y
        let x = s.root(cx, cx.alloc_copy(&[1.0, 2.0, 3.0]).unwrap().finish());
        let y = s.root(cx, cx.alloc_copy(&[10.0, 20.0, 30.0]).unwrap().finish());
        cx.edit_pair::<f64, f64, _>(x, y, |xs, ys| xs.iter_mut().zip(ys).for_each(|(a, b)| *a += b)).unwrap();
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[11.0, 22.0, 33.0][..]));
        // the same vector: reversed into itself, which needs the snapshot
        let n = cx.edit_pair::<f64, f64, _>(x, x, |xs, ys| {
            let n = ys.len();
            for i in 0..n {
                xs[i] = ys[n - 1 - i];
            }
            n
        });
        assert_eq!(n.unwrap(), 3);
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[33.0, 22.0, 11.0][..]));
        // a shared x is duplicated first, and y (the old x) is untouched
        let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
        l.set(0, x.bind(cx));
        l.set(1, x.bind(cx));
        s.root(cx, l.finish());
        let old = s.root(cx, x.bind(cx));
        cx.edit_pair::<f64, f64, _>(x, old, |xs, ys| xs[0] = ys[0] * 2.0).unwrap();
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().get(0), 66.0);
        assert_eq!(old.bind(cx).as_vector::<f64>().unwrap().get(0), 33.0);
        // other element types
        let i = s.root(cx, cx.alloc_copy(&[1, 2]).unwrap().finish());
        cx.edit_pair::<f64, i32, _>(x, i, |xs, ys| xs[2] = ys[1] as f64).unwrap();
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().get(2), 2.0);
    });
}

/// Roots, `Out` slots and argument windows count no reference (§4).
#[test]
fn roots_and_stack_slots_count_nothing() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = cx.alloc_copy(&[1]).unwrap().finish();
        let r = s.root(cx, x);
        let o = s.out(cx);
        o.set(cx, r.bind(cx));
        let _ = s.args(cx, &[r.bind(cx), r.bind(cx)], None);
        let _g = cx.global(r.bind(cx));
        assert_eq!(r.bind(cx).refcnt(), 0);
    });
}

/// Sol's R1b S2: OBJECT and S4 survive a duplicate with no attributes
/// (`duplicate1`'s common exit copies them whenever the types match).
#[test]
fn a_duplicate_keeps_object_and_s4_without_attributes() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_copy(&[1, 2]).unwrap().finish());
        // SAFETY: a live vector of this heap; OBJECT and S4 change no body.
        unsafe {
            let o = x.bind(cx).sexp();
            o.set_object(true);
            o.set_gp(o.gp() | (1 << 4));
        }
        let out = s.out(cx);
        let d = out.root();
        cx.shallow_duplicate(x, out).unwrap();
        assert_ne!(d.bind(cx), x.bind(cx));
        assert_eq!(d.bind(cx).attrib(), Some(cx.nil()));
        assert!(d.bind(cx).is_object() && d.bind(cx).is_s4());
    });
}

/// A pairlist of `n` cells whose cars are integers `0..n`, cell `i` tagged
/// `t` and carrying a `names`-like attribute pairlist when `i` is `marked`.
fn long_pairlist(cx: &mut Cx, n: usize, marked: usize) -> robigo_rtcore::Global {
    let cx = &*cx;
    let nil = cx.nil();
    let mut l = nil;
    for i in (0..n).rev() {
        let v = cx.alloc_copy(&[i as i32]).unwrap().finish();
        l = cx.cons(v.into(), l).unwrap();
        if i == marked {
            let attr = cx.cons(cx.install(b"label").unwrap().into(), nil).unwrap();
            // SAFETY: fresh cells of this heap, unpublished.
            unsafe {
                attr.sexp().set_tag(cx.install(b"note").unwrap().as_ref().sexp());
                l.sexp().set_tag(cx.install(b"t").unwrap().as_ref().sexp());
                l.sexp().set_attrib(attr.sexp());
            }
        }
    }
    cx.global(l)
}

/// Sol's R1b S1: a long pairlist is copied in chunks, cell for cell.
#[test]
fn a_long_pairlist_is_copied_in_chunks() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let g = long_pairlist(cx, 2500, 1500);
    cx.scope(|cx, s| {
        let x = s.root(cx, g.bind(cx));
        let out = s.out(cx);
        let d = out.root();
        cx.shallow_duplicate(x, out).unwrap();
        let (mut a, mut b) = (x.bind(cx), d.bind(cx));
        let mut n = 0;
        while let (Some(ca), Some(cb)) = (a.as_cons(), b.as_cons()) {
            assert_ne!(ca, cb, "cell {n} is new");
            assert_eq!((ca.car(), ca.tag()), (cb.car(), cb.tag()), "cell {n}");
            let (aa, ab) = (ca.as_ref().attrib().unwrap(), cb.as_ref().attrib().unwrap());
            if n == 1500 {
                let (aa, ab) = (aa.as_cons().unwrap(), ab.as_cons().unwrap());
                assert_ne!(aa, ab, "the attribute cell is copied");
                assert_eq!((aa.car(), aa.tag()), (ab.car(), ab.tag()));
            } else {
                assert_eq!(ab, cx.nil());
            }
            a = ca.cdr();
            b = cb.cdr();
            n += 1;
        }
        assert_eq!((n, b), (2500, cx.nil()));
    });
    cx.release_global(g);
}

/// The chunked copy polls: with a collection due, it is served during the
/// copy (by the per-chunk poll), not after it.
#[cfg(not(miri))]
#[test]
fn a_long_pairlist_copy_serves_a_due_collection() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    let g = long_pairlist(cx, 3000, 10);
    cx.scope(|cx, s| {
        let x = s.root(cx, g.bind(cx));
        make_due(cx);
        let before = by_kind(cx, Safepoint::PollAlloc);
        let out = s.out(cx);
        let d = out.root();
        cx.shallow_duplicate(x, out).unwrap();
        assert!(by_kind(cx, Safepoint::PollAlloc) > before);
        assert_eq!(d.bind(cx).len(), Some(3000));
    });
    cx.release_global(g);
}

/// Under Miri a collection panics with its reason (the collector calls the
/// heap's C shim) rather than failing inside it: the tests that collect are
/// `cfg(not(miri))`, and this one checks that the rule is enforced.
#[cfg(miri)]
#[test]
#[should_panic(expected = "a collection under Miri")]
fn under_miri_a_collection_panics_with_its_reason() {
    let mut iso = OwnedIsolate::new();
    let _ = iso.cx().need_gc(robigo_rtcore::NeedGc::new(REALSXP, 1));
}
