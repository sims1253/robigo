//! The rt-core API, tested as its users see it (docs/g3.md §2). The owner
//! and scope checks are enforced in every build, so CI runs these in
//! release too (§2.3: "M1 tests cross-isolate operations in release
//! builds, root reset with a live view, scope escape and nested native
//! re-entry"; the compile-time halves are in `compile_fail.rs`).

use robigo_rtcore::types::*;
use robigo_rtcore::{
    Char, Cx, Encoding, Exit, ExitKind, Lgl, Options, OwnedIsolate, RError, RResult, Ref, Root, Sexp, Slot, View,
};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn roots(cx: &Cx) -> Vec<Sexp> {
    let mut v = Vec::new();
    cx.for_each_root(&mut |x| v.push(x));
    v
}

/// The message of the panic `f` raises.
fn panic_message<R>(f: impl FnOnce() -> R) -> String {
    let Err(p) = catch_unwind(AssertUnwindSafe(f)) else { panic!("expected a panic") };
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .expect("a panic whose payload is not a string")
}

fn small() -> OwnedIsolate {
    OwnedIsolate::with_options(Options { value_stack_slots: 4096, ..Options::default() })
}

#[test]
fn handles_are_one_word() {
    let word = size_of::<usize>();
    assert_eq!(size_of::<Ref<'static>>(), word);
    assert_eq!(size_of::<RResult<Ref<'static>>>(), word);
    assert_eq!(size_of::<Root<'static>>(), word);
    assert_eq!(size_of::<RError>(), 0);
    // (an `align(16)` slot: 16 bytes whatever the word)
    assert_eq!(size_of::<Slot>(), 16);
}

#[test]
fn views_read_what_was_allocated() {
    let mut iso = small();
    let cx = iso.cx();
    assert!(matches!(cx.nil().view(), View::Null));
    assert_eq!(cx.nil().len(), Some(0));

    let x = cx.alloc_copy(&[1.5, 2.5]).unwrap().finish();
    assert_eq!(x.as_slice(), Some(&[1.5, 2.5][..]));
    assert!(matches!(x.as_ref().view(), View::Real(v) if v.len() == 2));
    assert_eq!(x.as_ref().attrib(), Some(cx.nil()));

    let i = cx.alloc_with(4, |i| i as i32 * 10).unwrap().finish();
    assert_eq!(i.get(3), 30);
    let l = cx.alloc_vector::<Lgl>(3).unwrap().finish();
    assert_eq!(l.as_slice(), Some(&[Lgl::FALSE; 3][..]));

    let a = cx.mk_char(b"abc", Encoding::Utf8).unwrap();
    assert_eq!(a.bytes(), b"abc");
    assert!(a.is_ascii());
    assert_eq!(a.enc(), Encoding::Native, "an ASCII string carries no mark");
    let e = cx.mk_char("é".as_bytes(), Encoding::Utf8).unwrap();
    assert_eq!(e.enc(), Encoding::Utf8);
    assert_eq!(cx.mk_char(b"abc", Encoding::Native), Ok(a), "CHARSXPs are cached");
    assert!(panic_message(|| cx.mk_char(b"a\0b", Encoding::Native)).contains("embedded NUL"));
    assert_eq!(cx.blank_string().bytes(), b"");
    assert_ne!(cx.na_string(), cx.mk_char(b"NA", Encoding::Native).unwrap());

    let mut s = cx.alloc_vector::<Char<'_>>(2).unwrap();
    assert_eq!(s.get(1), cx.blank_string());
    s.set(1, a);
    let s = s.finish();
    assert!(matches!(s.as_ref().view(), View::Str(v) if v.get(1) == a));

    let sym = cx.install(b"foo").unwrap();
    assert_eq!(sym.name().bytes(), b"foo");
    assert_eq!(cx.install(b"foo"), Ok(sym));
    assert!(matches!(sym.as_ref().view(), View::Sym(_)));

    let tail = cx.cons(s.into(), cx.nil()).unwrap();
    let call = cx.lcons(sym.into(), tail).unwrap();
    let View::Lang(c) = call.view() else { panic!("not a call") };
    assert_eq!(c.car(), sym.as_ref());
    assert_eq!(c.cdr().as_cons().unwrap().car(), s.as_ref());
    assert_eq!(c.tag(), cx.nil());
    assert_eq!(call.len(), Some(2));
    assert_eq!(cx.unbound().sexptype(), SYMSXP);
    assert_eq!(cx.missing_arg().sexptype(), SYMSXP);
}

#[test]
fn fresh_lists_count_their_edges_exactly() {
    let mut iso = small();
    let cx = iso.cx();
    let x = cx.alloc_copy(&[1.0]).unwrap().finish().as_ref();
    let y = cx.alloc_copy(&[2.0]).unwrap().finish().as_ref();
    let mut l = cx.alloc_vector::<Ref<'_>>(2).unwrap();
    l.set(0, x);
    l.set(1, x);
    assert_eq!(x.refcnt(), 2);
    assert!(x.maybe_shared());
    l.set(1, y);
    assert_eq!((x.refcnt(), y.refcnt()), (1, 1));
    l.set(1, y);
    assert_eq!(y.refcnt(), 1, "storing the same value again changes nothing");
    let l = l.finish();
    assert_eq!(l.get(0), x);
    assert!(!l.as_ref().maybe_referenced());
}

#[test]
fn roots_are_precise_roots_until_their_scope_ends() {
    let mut iso = small();
    let cx = iso.cx();
    let base = cx.stack_depth();
    let kept = cx.scope(|cx, s| {
        let x = cx.alloc_copy(&[1.0]).unwrap().finish();
        let r = s.root(cx, x);
        let addr = x.as_ref().sexp();
        assert!(roots(cx).contains(&addr));
        // the root survives a call that suspends the capability, and binds
        // afterwards to the same object (`x` itself cannot be used here)
        cx.call_out(|_| ());
        assert_eq!(r.bind(cx).sexp(), addr);
        cx.scope(|cx, inner| {
            let y = cx.alloc_copy(&[2.0]).unwrap().finish();
            inner.root(cx, y);
            assert_eq!(cx.stack_depth(), base + 2);
        });
        assert_eq!(cx.stack_depth(), base + 1, "the inner scope's root is gone");
        addr
    });
    assert_eq!(cx.stack_depth(), base);
    assert!(!roots(cx).contains(&kept));
}

#[test]
fn scopes_restore_the_stack_on_err_and_on_panic() {
    let mut iso = small();
    let cx = iso.cx();
    let base = cx.stack_depth();
    let r: RResult<()> = cx.scope(|cx, s| {
        s.root(cx, cx.nil());
        s.out(cx);
        Err(cx.error_call(None, "boom"))
    });
    let e = r.unwrap_err();
    assert_eq!(cx.stack_depth(), base);
    assert_eq!(cx.exit_info(&e).kind, ExitKind::Error);
    cx.resolve_exit(e, None);

    let caught = catch_unwind(AssertUnwindSafe(|| {
        cx.scope(|cx, s| {
            s.root(cx, cx.nil());
            panic!("inside a scope");
        })
    }));
    assert!(caught.is_err());
    assert_eq!(cx.stack_depth(), base);
    // and the isolate is usable again: a new scope opens and roots
    cx.scope(|cx, s| {
        s.root(cx, cx.nil());
    });
}

#[test]
fn an_out_writes_into_the_callers_slot() {
    fn callee(cx: &mut Cx, out: robigo_rtcore::Out<'_>) -> RResult<()> {
        cx.scope(|cx, s| {
            let tmp = s.root(cx, cx.alloc_copy(&[7]).unwrap().finish());
            out.set(cx, tmp.bind(cx)); // before the callee's roots go
            Ok(())
        })
    }
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let res = out.root();
        assert_eq!(res.bind(cx), cx.nil(), "an empty slot reads as NULL");
        callee(cx, out).unwrap();
        assert_eq!(res.bind(cx).as_vector::<i32>().unwrap().as_slice(), Some(&[7][..]));
        assert!(roots(cx).contains(&res.bind(cx).sexp()));
    });
}

#[test]
fn a_reset_root_is_empty_and_rewritable() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = cx.alloc_copy(&[1.0]).unwrap().finish();
        let r = s.root(cx, x);
        let xs = x.as_ref().sexp();
        let out = r.reset(cx);
        assert_eq!(r.bind(cx), cx.nil());
        assert!(!roots(cx).contains(&xs));
        out.set(cx, cx.install(b"y").unwrap());
        assert_eq!(r.bind(cx).sexptype(), SYMSXP);
    });
}

#[test]
fn args_are_a_window_with_names() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let a = cx.alloc_copy(&[1.0]).unwrap().finish().as_ref();
        let b = cx.install(b"b").unwrap().as_ref();
        let x = cx.install(b"x").unwrap();
        let args = s.args(cx, &[a, b], Some(&[Some(x), None]));
        assert_eq!(args.len(), 2);
        assert_eq!(args.get(cx, 0), a);
        assert_eq!(args.root(1).bind(cx), b);
        assert_eq!(args.name(cx, 0), Some(x));
        assert_eq!(args.name(cx, 1), None);
        assert!(!args.any_package_altrep());
        let unnamed = s.args(cx, &[a], None);
        assert_eq!(unnamed.name(cx, 0), None);
        assert!(panic_message(|| unnamed.get(cx, 1)).contains("argument 1 of 1"));
        assert_eq!(s.args(cx, &[], None).len(), 0);

        // a package ALTREP argument (the flag is all a builtin branches on),
        // in an ALTREP object's shape: node-sized, its data1, data2, class
        // and 4th word (+16, +24, +32, +40) the R_NilValue `cons` wrote
        let alt = cx.cons(cx.nil(), cx.nil()).unwrap();
        // SAFETY: `alt` is fresh and node-sized. Flagged ALTREP first, then
        // typed INTSXP, it is a well-formed ALTREP vector at every step: the
        // collector reads an ALTREP object's four node words as its edges,
        // all R_NilValue here, and nothing reads a length from it.
        unsafe {
            alt.sexp().set_flag(robigo_heap::flags::ALTREP, true);
            alt.sexp().set_typ(INTSXP);
        }
        assert!(matches!(alt.view(), View::Alt(_)));
        assert_eq!(alt.len(), None, "a package class's length needs &mut Cx");
        assert!(alt.as_vector::<i32>().is_none());
        assert!(s.args(cx, &[a, alt], None).any_package_altrep());
    });
}

#[test]
fn globals_outlive_scopes_until_released() {
    let mut iso = small();
    let cx = iso.cx();
    let g = cx.scope(|cx, _| cx.global(cx.alloc_copy(&[3.0]).unwrap().finish()));
    let x = g.bind(cx).sexp();
    assert!(roots(cx).contains(&x));
    assert_eq!(g.bind(cx).as_vector::<f64>().unwrap().get(0), 3.0);
    let g2 = cx.global(cx.nil());
    cx.release_global(g);
    assert!(!roots(cx).contains(&x));
    let g3 = cx.global(cx.install(b"z").unwrap()); // reuses the free entry
    assert_eq!(g3.bind(cx).sexptype(), SYMSXP);
    assert_eq!(g2.bind(cx), cx.nil());
}

#[test]
fn exits_carry_their_values_until_resolved() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let v = s.root(cx, cx.alloc_copy(&[42]).unwrap().finish());
        let mut exit = Exit::new(ExitKind::Jump);
        exit.info.mask = 4;
        exit.value = Some(v);
        let e = cx.exit(exit);
        assert_eq!(cx.pending_exits(), 1);
        assert!(roots(cx).iter().filter(|&&x| x == v.bind(cx).sexp()).count() >= 2);
        let call = s.root(cx, cx.lcons(cx.install(b"f").unwrap().into(), cx.nil()).unwrap());
        let e2 = cx.error_call(Some(call), "nested");
        assert_eq!(cx.pending_exits(), 2);
        let msg = s.out(cx);
        let msg_root = msg.root();
        let info = cx.resolve_exit(e2, Some(msg));
        assert_eq!(info.kind, ExitKind::Error);
        let m = msg_root.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        assert_eq!(m.bytes(), b"nested");
        let out = s.out(cx);
        let got = out.root();
        let info = cx.resolve_exit(e, Some(out));
        assert_eq!((info.kind, info.mask), (ExitKind::Jump, 4));
        assert_eq!(got.bind(cx), v.bind(cx));
        assert_eq!(cx.pending_exits(), 0);
    });
}

#[test]
fn an_allocation_the_heap_refuses_is_need_gc_then_gnu_rs_error() {
    let mut iso = small();
    let cx = iso.cx();
    let e = cx.alloc_vector::<f64>(usize::MAX / 4).map(|_| ()).unwrap_err();
    assert_eq!(e.sexptype(), REALSXP);
    let err = cx.need_gc(e).unwrap_err();
    cx.scope(|cx, s| {
        let out = s.out(cx);
        let m = out.root();
        cx.resolve_exit(err, Some(out));
        let m = m.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
        assert!(m.bytes().starts_with(b"cannot allocate vector of size "), "{:?}", m.bytes());
        assert!(m.bytes().ends_with(b" Gb"));
    });
}

// ---- owner checks: two isolates on one thread (release builds too) ----

#[test]
fn handles_and_objects_of_another_isolate_are_refused() {
    let mut a = small();
    let mut b = small();
    let ca = a.cx();
    let cb = b.cx();
    ca.scope(|ca, sa| {
        let xa = ca.alloc_copy(&[1.0]).unwrap().finish();
        let ra = sa.root(ca, xa);
        let args_a = sa.args(ca, &[xa.into()], None);
        let ga = ca.global(xa);
        cb.scope(|cb, sb| {
            // binding or resetting A's handles with B's capability
            assert!(panic_message(|| ra.bind(cb)).contains("another isolate"));
            assert!(panic_message(|| ra.reset(cb)).contains("another isolate"));
            assert!(panic_message(|| args_a.get(cb, 0)).contains("another isolate"));
            assert!(panic_message(|| ga.bind(cb)).contains("another isolate"));
            // rooting, publishing or storing A's object in B
            let xa = ra.bind(ca);
            assert!(panic_message(|| sb.root(cb, xa)).contains("another isolate"));
            assert!(panic_message(|| sb.args(cb, &[xa], None)).contains("another isolate"));
            assert!(panic_message(|| sb.out(cb).set(cb, xa)).contains("another isolate"));
            assert!(panic_message(|| cb.global(xa)).contains("another isolate"));
            assert!(panic_message(|| cb.cons(xa, cb.nil())).contains("another isolate"));
            assert!(panic_message(|| cb.alloc_vector::<Ref<'_>>(1).unwrap().set(0, xa)).contains("another isolate"));
            // A's scope token with B's capability
            assert!(panic_message(|| sa.root(cb, cb.nil())).contains("another isolate"));
            // B's Out written with A's capability
            let ob = sb.out(cb);
            assert!(panic_message(|| ob.set(ca, ca.nil())).contains("another isolate"));
            // and the right capability still works
            assert_eq!(sb.root(cb, cb.nil()).bind(cb), cb.nil());
        });
        assert_eq!(ra.bind(ca), xa.as_ref());
        ca.release_global(ga);
    });
    assert!(!ca.same_isolate(cb));
}

#[test]
fn a_global_is_released_only_by_its_isolate() {
    let mut a = small();
    let mut b = small();
    let ca = a.cx();
    let g = ca.global(ca.nil());
    let cb = b.cx();
    assert!(panic_message(|| cb.release_global(g)).contains("another isolate"));
}

#[test]
fn only_the_innermost_scope_takes_roots() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, outer| {
        outer.root(cx, cx.nil());
        cx.scope(|cx, _inner| {
            // the inner scope's end would drop a root pushed now
            assert!(panic_message(|| outer.root(cx, cx.nil())).contains("not the innermost"));
            assert!(panic_message(|| outer.out(cx)).contains("not the innermost"));
        });
        cx.scope(|_, _| ());
        // with no inner scope open, it takes roots again (nested scopes
        // opened at the same top are told apart)
        outer.root(cx, cx.nil());
    });
}

// ---- re-entry (a callback hook standing in for C until A1) ----

#[test]
fn nested_reentry_gets_fresh_capabilities_and_restores_the_stack() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        let depth = cx.stack_depth();
        let seen: Vec<usize> = cx.call_out(|re| {
            // "C" calls back in, which calls out again, three levels deep
            re.enter(|cx1| {
                let mut seen = vec![cx1.stack_depth()];
                cx1.scope(|cx1, s1| {
                    let y = s1.root(cx1, cx1.alloc_copy(&[2.0]).unwrap().finish());
                    // the outer root is still readable from the callback
                    assert_eq!(x.bind(cx1).as_vector::<f64>().unwrap().get(0), 1.0);
                    cx1.call_out(|re2| {
                        re2.enter(|cx2| {
                            seen.push(cx2.stack_depth());
                            cx2.scope(|cx2, s2| {
                                s2.root(cx2, cx2.nil());
                                cx2.call_out(|re3| re3.enter(|cx3| seen.push(cx3.stack_depth())));
                            });
                        })
                    });
                    assert_eq!(y.bind(cx1).as_vector::<f64>().unwrap().get(0), 2.0);
                });
                seen
            })
        });
        assert_eq!(seen, vec![depth, depth + 1, depth + 2]);
        assert_eq!(cx.stack_depth(), depth);
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().get(0), 1.0);
    });
}

#[test]
fn a_callback_needs_the_capability_suspended() {
    let mut iso = small();
    let cx = iso.cx();
    cx.call_out(|re| {
        re.enter(|_| {
            // a second capability while the first callback's runs
            assert!(
                panic_message(|| re.enter(|_| ())).contains("re-entry into an isolate whose capability is running")
            );
        });
        // sequential callbacks are fine
        re.enter(|_| ());
        re.enter(|_| ());
    });
    // a callback's roots and scopes end with it, even when it panics
    let depth = cx.stack_depth();
    cx.call_out(|re| {
        let r = catch_unwind(AssertUnwindSafe(|| {
            re.enter(|cx| {
                cx.scope(|cx, s| {
                    s.root(cx, cx.nil());
                    panic!("in a callback")
                })
            })
        }));
        assert!(r.is_err());
        re.enter(|cx| assert_eq!(cx.stack_depth(), depth));
    });
    assert_eq!(cx.stack_depth(), depth);
}

#[test]
fn a_callback_cannot_root_into_the_suspended_scope() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        cx.call_out(|re| {
            re.enter(|cx| {
                // the callback's end would drop it while `s` is still open
                assert!(panic_message(|| s.root(cx, cx.nil())).contains("not the innermost"));
            })
        });
        s.root(cx, cx.nil());
    });
}

#[test]
fn the_raw_entry_is_the_same_capability() {
    let mut iso = small();
    let cx = iso.cx();
    let nil = cx.nil().sexp();
    cx.call_out(|re| {
        let p = re.as_ptr();
        // SAFETY: the capability stays suspended for this call, on this thread.
        let got = unsafe { robigo_rtcore::enter_raw(p, |cx| cx.nil().sexp()) };
        assert_eq!(got, nil);
    });
}

// ---- the value stack ----

#[test]
fn a_full_value_stack_panics_and_recovers() {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1, ..Options::default() });
    let cx = iso.cx();
    let slots = cx.stack_headroom();
    assert!(slots >= 1);
    let msg = panic_message(|| {
        cx.scope(|cx, s| {
            for _ in 0..=slots {
                s.root(cx, cx.nil());
            }
        })
    });
    assert!(msg.contains("value stack is full"));
    assert_eq!(cx.stack_depth(), 0);
    assert_eq!(cx.stack_headroom(), slots);
}

/// The guard page after the reservation faults (in a child process, which
/// it kills with SIGSEGV).
#[cfg(all(target_os = "linux", not(miri)))]
#[test]
fn the_guard_page_faults() {
    let mut iso = small();
    let (_, hi) = iso.cx().stack_bounds();
    // SAFETY: the child only writes one byte and exits; it calls nothing
    // that could need a lock another thread of the test harness holds.
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0);
        if pid == 0 {
            std::ptr::write_volatile(hi as *mut u8, 1);
            libc::_exit(0);
        }
        let mut status = 0;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
        assert!(libc::WIFSIGNALED(status), "the write past the stack did not fault");
        assert_eq!(libc::WTERMSIG(status), libc::SIGSEGV);
    }
}

#[test]
fn roots_are_exactly_the_live_object_slots() {
    let mut iso = small();
    let cx = iso.cx();
    // the isolate's own roots (its environments and symbol cells)
    let own: HashSet<Sexp> = roots(cx).into_iter().collect();
    cx.scope(|cx, s| {
        let mut made = HashSet::new();
        for i in 0..100 {
            let x = cx.alloc_copy(&[i as f64]).unwrap().finish();
            made.insert(x.as_ref().sexp());
            s.root(cx, x);
            if i % 10 == 0 {
                s.out(cx); // empty slots are not roots
            }
        }
        let got: HashSet<Sexp> = roots(cx).into_iter().filter(|x| !own.contains(x)).collect();
        assert_eq!(got, made);
    });
    assert!(roots(cx).iter().all(|x| own.contains(x)));
}

// ---- review regressions ----

/// A second `Out` for one slot, made by `Root::reset`, cannot replace the
/// value the first wrote (Sol R1a S3).
#[test]
fn an_out_writes_its_slot_once() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let first = s.out(cx);
        let root = first.root();
        let second = root.reset(cx);
        first.set(cx, cx.nil());
        let y = s.root(cx, cx.alloc_copy(&[1]).unwrap().finish());
        assert!(panic_message(|| second.set(cx, y.bind(cx))).contains("already holds a value"));
        assert_eq!(root.bind(cx), cx.nil());
        root.reset(cx).set(cx, y.bind(cx));
        assert_eq!(root.bind(cx), y.bind(cx));
    });
}

#[test]
fn build_writes_the_result_or_records_the_failure() {
    let mut iso = small();
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_copy(&[1.0, 2.0]).unwrap().finish());
        let out = s.out(cx);
        let res = out.root();
        cx.build(out, |cx| {
            let xs = x.bind(cx).as_vector::<f64>().unwrap().as_slice().unwrap();
            Ok(cx.alloc_with(xs.len(), |i| xs[i] * 2.0)?.finish().into())
        })
        .unwrap();
        assert_eq!(res.bind(cx).as_vector::<f64>().unwrap().as_slice(), Some(&[2.0, 4.0][..]));

        let out = s.out(cx);
        let res = out.root();
        let mut runs = 0;
        let e = cx
            .build(out, |cx| {
                runs += 1;
                Ok(cx.alloc_vector::<f64>(usize::MAX / 4)?.finish().into())
            })
            .unwrap_err();
        assert_eq!(runs, 1, "a request past the address space is refused without a retry");
        assert_eq!(cx.exit_info(&e).kind, ExitKind::Error);
        cx.resolve_exit(e, None);
        assert_eq!(res.bind(cx), cx.nil(), "nothing was written");
    });
}

/// `need_gc` collects with the isolate's roots: under a vector memory limit
/// (80 MiB; GNU R's vector heap starts at 64) a refused allocation is served
/// after the garbage goes, and a rooted vector survives the collection. (Not under Miri: the collector calls the
/// heap's C shim.)
#[cfg(not(miri))]
#[test]
fn need_gc_collects_the_garbage_and_keeps_the_roots() {
    let mut iso = OwnedIsolate::with_options(Options {
        value_stack_slots: 4096,
        max_vsize: Some(80 << 20),
        ..Options::default()
    });
    let cx = iso.cx();
    cx.scope(|cx, s| {
        let x = s.root(cx, cx.alloc_with(1000, |i| i as f64).unwrap().finish());
        // garbage (256 KiB vectors, below LARGE) until the limit refuses one
        let refused = (0..1000).find_map(|_| cx.alloc_vector::<f64>(1 << 15).err());
        assert!(refused.is_some(), "the limit refused nothing");
        let out = s.out(cx);
        let res = out.root();
        let mut runs = 0;
        cx.build(out, |cx| {
            runs += 1;
            Ok(cx.alloc_vector::<f64>(1 << 15)?.finish().into())
        })
        .unwrap();
        assert_eq!(runs, 2, "refused, collected, retried");
        assert_eq!(res.bind(cx).len(), Some(1 << 15));
        assert_eq!(x.bind(cx).as_vector::<f64>().unwrap().get(999), 999.0);
    });
}
