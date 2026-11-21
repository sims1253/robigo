//! The boot heap's freeze (robigo-heap's `freeze.rs`), on an isolate: what
//! the freeze roots reach is frozen, the global environment's frame is
//! not, a finalizer on a frozen object unfreezes it at the next collection
//! (and runs once its last reference is gone), and the invariant holds
//! throughout.

#![allow(unsafe_code)]

use crate::OwnedIsolate;

#[test]
#[cfg(not(miri))]
fn a_frozen_object_with_a_finalizer_dies_after_its_last_reference_goes() {
    let mut iso = OwnedIsolate::new();
    let cx = iso.cx();
    if !cx.freeze_enabled() {
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
        let n = cx.freeze_boot_heap();
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
