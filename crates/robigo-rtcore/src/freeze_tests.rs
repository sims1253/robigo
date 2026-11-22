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
/// a collection (Sol S3). Its frozen referrer, the list, was made ordinary
/// with it, so only reachability decides; a log entry would have kept it.
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
        assert!(!cx.is_frozen(e.bind(cx)) && !cx.is_frozen(l.bind(cx)), "the key and its frozen referrer thawed");
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
