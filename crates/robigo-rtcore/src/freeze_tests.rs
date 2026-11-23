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
