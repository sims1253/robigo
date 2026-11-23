//! The boot heap's freeze (robigo-heap's `freeze.rs`), on an isolate: what
//! the freeze roots reach is frozen, the global environment's frame is
//! not, a finalizer on a frozen object unfreezes it at the next collection
//! (and runs once its last reference is gone), and the invariant holds
//! throughout.

#![allow(unsafe_code)]

use crate::OwnedIsolate;

/// The freeze on for this isolate (`ROBIGO_GC_FREEZE` is off by default),
/// unless a moving mode leaves it off.
fn freeze_on(cx: &mut crate::Cx) -> bool {
    cx.set_freeze_enabled(true);
    cx.freeze_enabled()
}

/// The boot boundary, then the collection that freezes (the lazy freeze).
fn freeze_now(cx: &mut crate::Cx) -> usize {
    cx.freeze_boot_heap();
    cx.collect();
    cx.frozen_count()
}

#[test]
#[cfg(not(miri))]
fn a_frozen_object_with_a_finalizer_dies_after_its_last_reference_goes() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        // (ROBIGO_GC_MOVE's modes leave the freeze off)
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let global = s.root(cx, cx.global_env());
        let sym = cx.install_rooted(s, b"freeze_test_env")?;
        let gsym = cx.install_rooted(s, b"freeze_test_global")?;
        // an environment bound in base (a boot object) and one in the global
        // environment (the session's)
        let o = s.out(cx);
        let e = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        cx.define_var(sym, e, base)?;
        let o = s.out(cx);
        let g = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        cx.define_var(gsym, g, global)?;
        let n = freeze_now(cx);
        assert!(n > 0, "the freeze froze nothing");
        assert!(cx.is_frozen(e.bind(cx)), "an environment base binds is frozen");
        assert!(!cx.is_frozen(g.bind(cx)), "the global environment's are not");
        cx.check_frozen();
        // a finalizer on the frozen environment: ordinary at the next collection
        let nil = cx.nil().sexp();
        // SAFETY: `e` is rooted; the reference is rooted by the collector.
        unsafe { cx.new_weak_ref_raw(e.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        assert!(!cx.is_frozen(e.bind(cx)), "the finalizer's key is ordinary after the next collection");
        cx.check_frozen();
        // its last reference gone: the next collection finds it dead
        let nilr = s.root(cx, cx.nil());
        cx.define_var(sym, nilr, base)?;
        let o = e.reset(cx);
        o.set(cx, cx.nil());
        cx.collect();
        assert!(cx.finalizers_pending(), "the unfrozen environment died, and its finalizer is ready");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// A frozen list that base binds holds an environment, a finalizer is put
/// on the environment, and the list's binding goes: the environment dies at
/// a collection (Sol S3). The next collection's frozen-reachability pass
/// thaws the environment; the list stays frozen and logged while base binds
/// it (the environment is reachable then, in GNU R too). Rebinding the base
/// binding is a dirty event, so the next pass thaws the list, and only
/// reachability decides: no log entry keeps the environment.
#[test]
#[cfg(not(miri))]
fn a_frozen_container_does_not_keep_an_unfrozen_object_alive() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let sym = cx.install_rooted(s, b"freeze_test_list")?;
        let o = s.out(cx);
        let e = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let o = s.out(cx);
        let l = o.root();
        cx.build(o, |cx| cx.cons(e.bind(cx), cx.nil()))?;
        cx.define_var(sym, l, base)?;
        freeze_now(cx);
        assert!(cx.is_frozen(e.bind(cx)) && cx.is_frozen(l.bind(cx)));
        let nil = cx.nil().sexp();
        // SAFETY: `e` is rooted; the reference is rooted by the collector.
        unsafe { cx.new_weak_ref_raw(e.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        assert!(!cx.is_frozen(e.bind(cx)), "the key thawed");
        assert!(cx.is_frozen(l.bind(cx)), "its referrer stays frozen (logged) while base binds it");
        cx.check_frozen();
        let nilr = s.root(cx, cx.nil());
        cx.define_var(sym, nilr, base)?;
        let o = e.reset(cx);
        o.set(cx, cx.nil());
        let o = l.reset(cx);
        o.set(cx, cx.nil());
        cx.collect();
        assert!(cx.finalizers_pending(), "the environment died with its list");
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// A global lookup fills a symbol's global cache with a binding cell of the
/// global environment before the freeze: the cell is not frozen (Sol S2).
#[test]
#[cfg(not(miri))]
fn a_global_cache_entry_does_not_freeze_the_global_environment() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let global = s.root(cx, cx.global_env());
        let sym = cx.install_rooted(s, b"freeze_test_cached")?;
        let o = s.out(cx);
        let v = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        cx.define_var(sym, v, global)?;
        let o = s.out(cx);
        let found = o.root();
        cx.find_var(sym, global, o)?;
        assert_eq!(found.bind(cx).sexp(), v.bind(cx).sexp());
        freeze_now(cx);
        assert!(!cx.is_frozen(v.bind(cx)), "a value only the global environment binds is not frozen");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// A profile's `parent.env(globalenv()) <- e`, with e's parent the global
/// environment, leads the search path back to the global environment before
/// the freeze: the walk stops there, and the global environment is not
/// frozen (#959's review).
#[test]
#[cfg(not(miri))]
fn a_search_path_led_back_to_the_global_environment_does_not_freeze_it() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let global = s.root(cx, cx.global_env());
        let o = s.out(cx);
        let e = o.root();
        cx.build(o, |cx| cx.new_env(cx.global_env(), true, 0))?;
        // the search-path walk itself (Sol r6 S12): the global environment
        // is offered as often after the rewiring as before it (the C API's
        // constants offer it, and the freeze's predicate refuses it there)
        let offered = |cx: &mut crate::Cx| {
            struct Count(robigo_heap::Sexp, usize);
            impl robigo_heap::Visitor for Count {
                fn edge(&mut self, x: &mut robigo_heap::Sexp) {
                    if *x == self.0 {
                        self.1 += 1;
                    }
                }
            }
            let iso = cx.iso();
            let mut c = Count(iso.envs().global_raw(), 0);
            // SAFETY: `&mut Cx`: no other reference into the roots is live,
            // and the visitor only reads.
            unsafe { iso.roots_mut() }.trace_freeze_roots(&mut c);
            c.1
        };
        let before = offered(cx);
        cx.set_enclos(global, e);
        assert_eq!(offered(cx), before, "the walk offers the global environment again");
        freeze_now(cx);
        assert!(cx.is_frozen(e.bind(cx)), "the attached environment is a freeze root");
        assert!(!cx.is_frozen(global.bind(cx)), "the global environment is not frozen");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// A frame pooled before the boot boundary is not handed out after it
/// (Sol r9 S15): it would be a session's frame at a boot object's address,
/// and the lazy freeze would make it immortal.
#[test]
#[cfg(not(miri))]
fn a_frame_pooled_before_the_boundary_is_not_reused_after_it() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let sh = s.root(cx, cx.new_shape(&[]).expect("room"));
        let g = s.root(cx, cx.global_env());
        // the pool works: a recycled frame is the next one of its capacity
        let out = s.out(cx);
        let a = out.root();
        cx.new_closure_frame(sh, g, g, &[], out)?;
        let a_addr = a.bind(cx).sexp().addr();
        cx.cleanup_envir(a, None);
        cx.recycle_frame(a, None);
        let out = s.out(cx);
        let b = out.root();
        cx.new_closure_frame(sh, g, g, &[], out)?;
        let b_addr = b.bind(cx).sexp().addr();
        // (ROBIGO_FRAME_QUARANTINE=1 holds every recycled frame back: none
        // is reused, before the boundary or after it)
        let quarantine = std::env::var("ROBIGO_FRAME_QUARANTINE").is_ok_and(|v| !v.is_empty() && v != "0");
        if quarantine {
            assert_ne!(b_addr, a_addr, "a quarantined frame was handed out again");
        } else {
            assert_eq!(b_addr, a_addr, "the pool hands the recycled frame out again");
        }
        cx.cleanup_envir(b, None);
        cx.recycle_frame(b, None);
        // across the boundary it does not
        cx.freeze_boot_heap();
        let out = s.out(cx);
        let c = out.root();
        cx.new_closure_frame(sh, g, g, &[], out)?;
        assert_ne!(c.bind(cx).sexp().addr(), b_addr, "a frame pooled before the boundary was reused after it");
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// The deferred search-path test (the pass's bar, (b)): an environment
/// attached before the freeze is frozen; attaching and detaching an
/// ordinary one in front of it keeps it on the search path, so it stays
/// frozen; detaching it takes it off, and the next collection's pass thaws
/// it. The test and the freeze roots walk the search path with one function
/// (`Roots::trace_search_path`).
#[test]
#[cfg(not(miri))]
fn a_detached_frozen_environment_thaws_and_one_still_attached_does_not() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let global = s.root(cx, cx.global_env());
        // attach `f` behind the global environment, as attach() does
        // SAFETY: the global environment is live, and so is its ENCLOS.
        let old = s.root(cx, unsafe { crate::Ref::from_sexp(robigo_heap::env::enclos(global.bind(cx).sexp()), cx) });
        let o = s.out(cx);
        let f = o.root();
        cx.build(o, |cx| cx.new_env(old.bind(cx), true, 0))?;
        cx.set_enclos(global, f);
        freeze_now(cx);
        assert!(cx.is_frozen(f.bind(cx)), "an attached environment is a freeze root");
        // an ordinary environment attached in front of it and detached,
        // attach()'s and detach()'s order of writes, several times: the path
        // is written each time, and its frozen members never change, so no
        // pass runs (Sol r1 S7)
        cx.collect();
        let passes = cx.frozen_passes();
        for _ in 0..3 {
            let o = s.out(cx);
            let u = o.root();
            cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
            cx.set_enclos(global, u);
            cx.set_enclos(u, f);
            cx.collect();
            assert!(cx.is_frozen(f.bind(cx)), "still on the search path behind an attached one: still frozen");
            cx.set_enclos(global, f);
            cx.collect();
            assert!(cx.is_frozen(f.bind(cx)), "and after it is detached");
        }
        assert_eq!(cx.frozen_passes(), passes, "attaching and detaching in front of it ran no pass");
        cx.check_frozen();
        // `f` detached behind an ordinary environment that stays attached:
        // the global environment is not the one written
        let o = s.out(cx);
        let u = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        cx.set_enclos(global, u);
        cx.set_enclos(u, f);
        cx.collect();
        assert!(cx.is_frozen(f.bind(cx)));
        cx.set_enclos(u, old);
        cx.collect();
        assert!(!cx.is_frozen(f.bind(cx)), "a frozen environment detached behind an attached one thaws");
        assert_eq!(cx.frozen_passes(), passes + 1, "one pass, for the one loss");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// A forced argument of a frozen closure frame, rebound: the slot's words
/// are cleared raw (`retire_arg`), so the cleared expression and value are
/// noted, and the next collection's pass thaws them (Sol r2 S9). Under the
/// check, I2 would report them otherwise.
#[test]
#[cfg(not(miri))]
fn a_retired_argument_of_a_frozen_frame_thaws() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let x = cx.install(b"freeze_test_arg").unwrap();
        let sh = s.root(cx, cx.new_shape(&[x]).expect("room"));
        let expr = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        let value = s.root(cx, cx.alloc_copy(&[2.0]).unwrap().finish());
        let o = s.out(cx);
        let frame = o.root();
        cx.new_closure_frame(sh, base, base, &[crate::SlotInit::Forced { expr, value }], o)?;
        let fsym = cx.install_rooted(s, b"freeze_test_frame")?;
        cx.define_var(fsym, frame, base)?;
        freeze_now(cx);
        assert!(cx.is_frozen(frame.bind(cx)) && cx.is_frozen(value.bind(cx)) && cx.is_frozen(expr.bind(cx)));
        // the argument rebound in the frozen frame
        let xsym = cx.install_rooted(s, b"freeze_test_arg")?;
        let nil = s.root(cx, cx.nil());
        cx.define_var(xsym, nil, frame)?;
        let o = value.reset(cx);
        o.set(cx, cx.nil());
        let o = expr.reset(cx);
        o.set(cx, cx.nil());
        let before = cx.frozen_passes();
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "the cleared argument words were noted");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// The step 2 bar's (a): a global filled since the last decision of the
/// frozen set (the freeze, a pass) is released without a note, so no pass
/// runs (the first JIT compile's temporary handle on a frozen builtin); one
/// filled before it is noted when released, and the next collection runs a
/// pass that thaws what only it held.
#[test]
#[cfg(not(miri))]
fn a_global_filled_since_the_last_decision_is_released_without_a_note() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        // bound in base: frozen, and stays reachable from base
        let kept = s.root(cx, cx.alloc_copy(&[1.0]).unwrap().finish());
        let ksym = cx.install_rooted(s, b"freeze_test_kept")?;
        cx.define_var(ksym, kept, base)?;
        // held only by a global filled before the freeze: frozen through it
        let held = cx.alloc_copy(&[2.0]).unwrap().finish();
        let old = cx.global(held);
        freeze_now(cx);
        assert!(cx.is_frozen(kept.bind(cx)));
        // filled after the freeze, released: no note, no pass
        let young = cx.global(kept.bind(cx));
        cx.release_global(young);
        let before = cx.frozen_passes();
        cx.collect();
        assert_eq!(cx.frozen_passes(), before, "a global no decision saw needs no note");
        // filled before the freeze, released: noted, one pass
        cx.release_global(old);
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "a global the freeze saw is noted");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// The step 2 bar's (b): the environment base's promises are to be
/// evaluated in (under the rdb boot, `..lazyLoad`'s frame) is a pinned
/// frame from the boot boundary, so forcing one of its promises, which
/// clears the promise's PRENV over that frozen environment, needs no pass
/// while another promise still uses it. The last one's clear is noted, and
/// the next pass lets the frame and what only it holds die, as in GNU R:
/// a finalizer on an environment the frame binds becomes ready (Sol step 2
/// r4 S15).
#[test]
#[cfg(not(miri))]
fn a_lazy_load_frame_is_exempt_until_its_last_promise_is_forced() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let lazy = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        // an environment only the frame holds, with a finalizer later
        let o = s.out(cx);
        let child = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let csym = cx.install_rooted(s, b"freeze_test_child")?;
        cx.define_var(csym, child, lazy)?;
        let expr = s.root(cx, cx.alloc_copy(&[3.0]).unwrap().finish());
        let p = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let q = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let psym = cx.install_rooted(s, b"freeze_test_lazy")?;
        let qsym = cx.install_rooted(s, b"freeze_test_lazy_q")?;
        cx.define_var(psym, p, base)?;
        cx.define_var(qsym, q, base)?;
        // what `makeLazy(vars, vals, expr, lazy, baseenv())` records while
        // base's file is read
        cx.while_reading_base_file(|cx| {
            cx.note_lazy_load_frame(lazy, base);
        });
        freeze_now(cx);
        assert!(cx.is_frozen(lazy.bind(cx)) && cx.is_frozen(p.bind(cx)) && cx.is_frozen(child.bind(cx)));
        // a finalizer on the child: ordinary at the next collection
        let nil = cx.nil().sexp();
        // SAFETY: a rooted environment of this isolate as the key.
        unsafe { cx.new_weak_ref_raw(child.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        let o = child.reset(cx);
        o.set(cx, cx.nil());
        let before = cx.frozen_passes();
        // what forcing it does to PRENV (`force_promise`: SET_PRENV(R_NilValue),
        // through the store barrier); rtcore has no evaluator to force it
        // SAFETY: a rooted promise of this isolate, and its nil.
        unsafe { p.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before, "another promise still uses the frame: no pass");
        assert!(!cx.finalizers_pending(), "the frame still holds the child");
        // the last promise forced
        // SAFETY: as above.
        unsafe { q.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        let o = lazy.reset(cx);
        o.set(cx, cx.nil());
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "the last promise's clear is noted");
        cx.collect();
        assert!(cx.finalizers_pending(), "the frame died, and the child with it");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// Sol step 2 r1 S2, S3: only `makeLazy`'s frame for base is pinned (another
/// assign environment's is not); a second boundary call keeps the pins; a
/// weak reference keyed on a pinned environment unpins it, so its next
/// PRENV clear is noted again and a pass decides it.
#[test]
#[cfg(not(miri))]
fn only_base_lazy_load_frames_are_pinned_and_a_weak_key_unpins_one() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        fn mk<'s>(
            cx: &mut crate::Cx,
            s: crate::Scope<'s>,
            name: &[u8],
        ) -> Result<(crate::Root<'s>, crate::Root<'s>), crate::RError> {
            let o = s.out(cx);
            let env = o.root();
            cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
            let expr = s.root(cx, cx.alloc_copy(&[3.0]).unwrap().finish());
            let p = s.root(cx, cx.mk_promise(expr.bind(cx), env.bind(cx)).expect("room"));
            let sym = cx.install_rooted(s, name)?;
            let base = s.root(cx, cx.base_env());
            cx.define_var(sym, p, base)?;
            Ok((env, p))
        }
        let (lazy, p) = mk(cx, s, b"freeze_test_lazy2")?;
        // a second promise on the frame: the first one's clear is exempt
        let expr2 = s.root(cx, cx.alloc_copy(&[4.0]).unwrap().finish());
        let p2 = s.root(cx, cx.mk_promise(expr2.bind(cx), lazy.bind(cx)).expect("room"));
        let p2sym = cx.install_rooted(s, b"freeze_test_lazy2b")?;
        cx.define_var(p2sym, p2, base)?;
        let (other, q) = mk(cx, s, b"freeze_test_other")?;
        let elsewhere = s.root(cx, cx.global_env());
        let (late, r) = mk(cx, s, b"freeze_test_late")?;
        cx.while_reading_base_file(|cx| {
            cx.note_lazy_load_frame(other, elsewhere);
            cx.note_lazy_load_frame(lazy, base);
        });
        // base's file no longer read (the system profile's makeLazy): not
        // pinned (Sol step 2 r3 S12)
        cx.note_lazy_load_frame(late, base);
        freeze_now(cx);
        // a second boundary call is refused and changes nothing (S3)
        cx.freeze_boot_heap();
        let before = cx.frozen_passes();
        // SAFETY: rooted promises of this isolate, and its nil.
        unsafe { p.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before, "the base lazy-load frame stays pinned");
        // SAFETY: as above.
        unsafe { q.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "another assign environment's frame is not pinned (S2)");
        let before = cx.frozen_passes();
        // SAFETY: as above.
        unsafe { r.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "a frame noted outside base's file is not pinned (S12)");
        // a weak reference keyed on the pinned frame unpins it (S2)
        let nil = cx.nil().sexp();
        // SAFETY: a rooted environment of this isolate as the key.
        unsafe { cx.new_weak_ref_raw(lazy.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        assert!(!cx.is_frozen(lazy.bind(cx)), "keyed by a weak reference: unpinned and thawed");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// Sol step 2 r2 S7: a weak reference keyed on a pinned lazy-load frame
/// between the boot boundary and the freeze unpins it (the frame is still
/// ordinary then), so once its last reference goes it dies and its
/// finalizer is ready.
#[test]
#[cfg(not(miri))]
fn a_weak_key_registered_before_the_freeze_unpins_a_lazy_load_frame() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let lazy = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let expr = s.root(cx, cx.alloc_copy(&[3.0]).unwrap().finish());
        let p = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let psym = cx.install_rooted(s, b"freeze_test_lazy3")?;
        cx.define_var(psym, p, base)?;
        cx.while_reading_base_file(|cx| {
            cx.note_lazy_load_frame(lazy, base);
        });
        cx.freeze_boot_heap();
        // keyed before the first collection: the frame is ordinary yet
        let nil = cx.nil().sexp();
        // SAFETY: a rooted environment of this isolate as the key.
        unsafe { cx.new_weak_ref_raw(lazy.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        assert!(!cx.is_frozen(lazy.bind(cx)), "a weak key is never frozen");
        // its last references gone: the promise's binding and our roots
        let nilr = s.root(cx, cx.nil());
        cx.define_var(psym, nilr, base)?;
        let o = p.reset(cx);
        o.set(cx, cx.nil());
        let o = lazy.reset(cx);
        o.set(cx, cx.nil());
        cx.collect();
        assert!(cx.finalizers_pending(), "unpinned, the frame died and its finalizer is ready");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// Sol step 2 r5 S18: only the promises base binds count for a lazy-load
/// frame's exemption. A promise the frame binds itself (a cycle) does not,
/// so clearing base's one promise's PRENV is noted, and the next pass lets
/// the frame, its own promise and the child it holds die.
#[test]
#[cfg(not(miri))]
fn a_promise_the_frame_binds_does_not_count_for_its_exemption() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let lazy = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let o = s.out(cx);
        let child = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let csym = cx.install_rooted(s, b"freeze_test_child2")?;
        cx.define_var(csym, child, lazy)?;
        let expr = s.root(cx, cx.alloc_copy(&[3.0]).unwrap().finish());
        let p = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let q = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let psym = cx.install_rooted(s, b"freeze_test_cycle_p")?;
        let qsym = cx.install_rooted(s, b"freeze_test_cycle_q")?;
        cx.define_var(psym, p, base)?;
        // q in the frame itself: the frame and q form a cycle
        cx.define_var(qsym, q, lazy)?;
        cx.while_reading_base_file(|cx| {
            cx.note_lazy_load_frame(lazy, base);
        });
        freeze_now(cx);
        assert!(cx.is_frozen(lazy.bind(cx)) && cx.is_frozen(q.bind(cx)));
        let nil = cx.nil().sexp();
        // SAFETY: a rooted environment of this isolate as the key.
        unsafe { cx.new_weak_ref_raw(child.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        cx.collect();
        for r in [child, lazy, q] {
            let o = r.reset(cx);
            o.set(cx, cx.nil());
        }
        let before = cx.frozen_passes();
        // SAFETY: a rooted promise of this isolate, and its nil.
        unsafe { p.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "base's only counted promise cleared: noted");
        cx.collect();
        assert!(cx.finalizers_pending(), "the frame, its own promise and the child died");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}

/// Sol step 2 r6 S22: the cycle of S18 with no pass between the freeze and
/// the clear, so only phase 1's count decides: the child is a weak key
/// before the freeze (never frozen; the frame is logged), and clearing
/// base's one promise must be noted at once.
#[test]
#[cfg(not(miri))]
fn phase_one_does_not_count_a_promise_the_frame_binds() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !freeze_on(cx) {
        return;
    }
    cx.scope(|cx, s| {
        let base = s.root(cx, cx.base_env());
        let o = s.out(cx);
        let lazy = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let o = s.out(cx);
        let child = o.root();
        cx.build(o, |cx| cx.new_env(cx.base_env(), true, 0))?;
        let csym = cx.install_rooted(s, b"freeze_test_child3")?;
        cx.define_var(csym, child, lazy)?;
        let nil = cx.nil().sexp();
        // SAFETY: a rooted environment of this isolate as the key.
        unsafe { cx.new_weak_ref_raw(child.bind(cx).sexp(), nil, cx.base_env().sexp(), false) }.expect("room");
        let expr = s.root(cx, cx.alloc_copy(&[3.0]).unwrap().finish());
        let p = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let q = s.root(cx, cx.mk_promise(expr.bind(cx), lazy.bind(cx)).expect("room"));
        let psym = cx.install_rooted(s, b"freeze_test_cycle3_p")?;
        let qsym = cx.install_rooted(s, b"freeze_test_cycle3_q")?;
        cx.define_var(psym, p, base)?;
        cx.define_var(qsym, q, lazy)?;
        cx.while_reading_base_file(|cx| {
            cx.note_lazy_load_frame(lazy, base);
        });
        let before = cx.frozen_passes();
        freeze_now(cx);
        assert!(cx.is_frozen(lazy.bind(cx)) && cx.is_frozen(q.bind(cx)) && !cx.is_frozen(child.bind(cx)));
        assert_eq!(cx.frozen_passes(), before, "no pass yet: phase 1's count stands");
        for r in [child, lazy, q] {
            let o = r.reset(cx);
            o.set(cx, cx.nil());
        }
        // SAFETY: a rooted promise of this isolate, and its nil.
        unsafe { p.bind(cx).sexp().set_tag(cx.nil().sexp()) };
        cx.collect();
        assert_eq!(cx.frozen_passes(), before + 1, "phase 1 counted base's promise only: the clear is noted");
        cx.collect();
        assert!(cx.finalizers_pending(), "the cycle died, and the child with it");
        cx.check_frozen();
        Ok::<(), crate::RError>(())
    })
    .unwrap();
}
