//! robigo-core's heap, strings, symbols and unwinding, tested directly.

use robigo_core::types::*;
use robigo_core::{Core, Sexp, flags, gp};
use std::cell::RefCell;

thread_local! {
    static ROOTS: RefCell<Vec<Sexp>> = const { RefCell::new(Vec::new()) };
}

unsafe fn mark_roots(c: &mut Core) {
    let roots = ROOTS.with(|r| r.borrow().clone());
    for r in roots {
        c.mark(r);
    }
}

fn core_with_roots() -> Box<Core> {
    ROOTS.with(|r| r.borrow_mut().clear());
    let mut c = Core::new();
    c.root_fns.push(mark_roots);
    c
}

fn root(x: Sexp) {
    ROOTS.with(|r| r.borrow_mut().push(x));
}

fn int_vec(c: &mut Core, vals: &[i32]) -> Sexp {
    let x = c.alloc_vector(INTSXP, vals.len());
    unsafe {
        std::ptr::copy_nonoverlapping(vals.as_ptr(), x.std_data() as *mut i32, vals.len());
    }
    x
}

unsafe fn ints<'a>(x: Sexp) -> &'a [i32] {
    unsafe { std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize) }
}

#[test]
fn constants() {
    let c = Core::new();
    unsafe {
        assert_eq!(c.nil.typ(), NILSXP);
        assert_eq!(c.nil.car(), c.nil);
        assert_eq!(c.nil.attrib_raw(), c.nil);
        assert!(c.nil.has_flag(flags::IMMORTAL));
        assert_eq!(c.na_string.char_bytes(), b"NA");
        assert_eq!(c.blank_string.char_bytes(), b"");
        assert_eq!(c.unbound.typ(), SYMSXP);
        assert_ne!(c.unbound, c.missing_arg);
    }
}

#[test]
fn vectors_are_initialized_like_gnu_r() {
    let mut c = Core::new();
    let s = c.alloc_vector(STRSXP, 3);
    let l = c.alloc_vector(VECSXP, 2);
    let r = c.alloc_vector(REALSXP, 1000);
    unsafe {
        let sd = s.std_data() as *const Sexp;
        assert!((0..3).all(|i| *sd.add(i) == c.blank_string));
        let ld = l.std_data() as *const Sexp;
        assert!((0..2).all(|i| *ld.add(i) == c.nil));
        assert_eq!(r.std_len(), 1000);
        assert_eq!(r.std_data() as usize % 16, 0);
    }
}

#[test]
fn chars_are_interned_by_bytes_and_encoding() {
    let mut c = Core::new();
    let a = c.mk_char(b"hello", 0);
    let b = c.mk_char(b"hello", gp::UTF8);
    assert_eq!(a, b, "ASCII drops the encoding bits");
    unsafe {
        assert!(a.gp() & gp::ASCII != 0);
        assert_eq!(*a.std_data().add(5), 0, "NUL-terminated");
    }
    let u = c.mk_char("café".as_bytes(), gp::UTF8);
    let l = c.mk_char("café".as_bytes(), gp::LATIN1);
    assert_ne!(u, l);
    assert_eq!(u, c.mk_char("café".as_bytes(), gp::UTF8));
    assert_ne!(c.mk_char(b"NA", 0), c.na_string);
}

#[test]
fn symbols_are_unique_and_immortal() {
    let mut c = Core::new();
    let x = c.install(b"x");
    assert_eq!(x, c.install(b"x"));
    assert_ne!(x, c.install(b"y"));
    unsafe {
        assert_eq!(x.typ(), SYMSXP);
        assert_eq!(x.car().char_bytes(), b"x");
        assert!(x.has_flag(flags::IMMORTAL));
    }
    for _ in 0..3 {
        c.collect();
    }
    unsafe { assert_eq!(c.install(b"x").car().char_bytes(), b"x") };
}

#[test]
fn rooted_objects_survive_and_garbage_is_freed() {
    let mut c = core_with_roots();
    let keep = int_vec(&mut c, &[1, 2, 3]);
    root(keep);
    let mut list = c.nil;
    for i in 0..1000 {
        let v = int_vec(&mut c, &[i]);
        list = c.alloc_node(LISTSXP, v, list, c.nil);
    }
    root(list);
    // garbage
    for i in 0..100_000 {
        int_vec(&mut c, &[i, i]);
    }
    c.collect();
    let after = c.stats.live_objects;
    unsafe {
        assert_eq!(ints(keep), &[1, 2, 3]);
        let mut n = 0;
        let mut p = list;
        while p != c.nil {
            assert_eq!(ints(p.car()), &[999 - n]);
            n += 1;
            p = p.cdr();
        }
        assert_eq!(n, 1000);
    }
    assert!(after < 10_000, "garbage freed: {after} live");
    assert!(c.stats.collections >= 2, "allocation triggers collections");
}

#[test]
fn stack_references_keep_objects_alive() {
    let mut c = Core::new();
    let x = int_vec(&mut c, &[42; 64]);
    // only an interior pointer survives in a local
    let interior = std::hint::black_box(unsafe { (x.std_data() as *mut i32).add(10) });
    let x_addr = x.addr();
    drop(x);
    for i in 0..200_000 {
        int_vec(&mut c, &[i]);
    }
    c.collect();
    assert_eq!(c.resolve(interior as usize).map(|s| s.addr()), Some(x_addr));
    unsafe { assert_eq!(*interior, 42) };
}

#[test]
fn dead_chars_leave_the_cache() {
    let mut c = Core::new();
    let n0 = {
        for i in 0..50_000 {
            c.mk_char(format!("s{i}").as_bytes(), 0);
        }
        c.collect();
        c.collect();
        c.stats.live_objects
    };
    assert!(n0 < 20_000, "{n0}");
    // a fresh string gets a fresh object
    let a = c.mk_char(b"s123", 0);
    unsafe { assert_eq!(a.char_bytes(), b"s123") };
}

#[test]
fn preserve_and_release() {
    let mut c = Core::new();
    let x = c.alloc_vector(REALSXP, 4);
    let addr = x.addr();
    c.preserve(x);
    for _ in 0..3 {
        c.collect();
    }
    assert!(c.resolve(addr).is_some());
    c.release(x);
}

static RELEASED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
unsafe extern "C" fn count_release(owner: usize) {
    RELEASED.fetch_add(owner, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn extdata_release_runs_once_dead() {
    let mut c = core_with_roots();
    let mut buf = vec![1.5f64, 2.5];
    let live = c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1000, Some(count_release));
    root(live);
    for _ in 0..10 {
        c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1, Some(count_release));
    }
    // overwrite stack slots that may still hold the dead objects
    std::hint::black_box([0usize; 64]);
    for _ in 0..3 {
        c.collect();
    }
    let r = RELEASED.load(std::sync::atomic::Ordering::SeqCst);
    assert!(r <= 10 && r >= 5, "dead extdata released ({r}), live one not");
    unsafe {
        assert!(live.has_flag(flags::EXTDATA));
        assert_eq!(*(live.std_data() as *const f64).add(1), 2.5);
    }
}

#[test]
fn weak_refs_queue_finalizers_of_dead_keys() {
    let mut c = core_with_roots();
    let key_live = c.alloc_vector(INTSXP, 1);
    root(key_live);
    let mk = |c: &mut Core, key: Sexp| {
        let w = c.alloc_vector(WEAKREFSXP, 4);
        let fin = c.alloc_vector(RAWSXP, 8);
        unsafe {
            let d = w.std_data() as *mut Sexp;
            *d = key;
            *d.add(1) = c.nil;
            *d.add(2) = fin;
        }
        c.register_weak_ref(w);
        w
    };
    let w1 = mk(&mut c, key_live);
    root(w1);
    let dead_key = c.alloc_vector(INTSXP, 1);
    let _w2 = mk(&mut c, dead_key);
    let dead_addr = dead_key.addr();
    drop(dead_key);
    std::hint::black_box([0usize; 64]);
    c.collect();
    let q = c.take_finalizable();
    assert!(q.len() <= 1);
    if let Some(&w) = q.first() {
        unsafe {
            assert_eq!((*(w.std_data() as *const Sexp)).addr(), dead_addr, "key kept for the finalizer");
            assert!(w.gp() & gp::READY_TO_FINALIZE != 0);
        }
    }
    assert_eq!(c.weak_refs().len(), 2 - q.len());
}

#[test]
fn try_run_and_jump() {
    let ok = robigo_core::unwind::try_run(|| {});
    assert!(ok);
    let mut reached = false;
    let r = &mut reached as *mut bool;
    let ok = robigo_core::unwind::try_run(move || unsafe {
        robigo_core::unwind::jump();
        #[allow(unreachable_code)]
        {
            *r = true;
        }
    });
    assert!(!ok);
    assert!(!reached);
    // nested: the inner context catches, the outer completes
    let ok = robigo_core::unwind::try_run(|| {
        assert_eq!(robigo_core::unwind::depth(), 1);
        let inner = robigo_core::unwind::try_run(|| unsafe { robigo_core::unwind::jump() });
        assert!(!inner);
    });
    assert!(ok);
    assert_eq!(robigo_core::unwind::depth(), 0);
}
