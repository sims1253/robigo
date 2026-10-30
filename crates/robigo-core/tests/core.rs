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
    let _ = x;
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
    // release callbacks wait for a safe point
    assert_eq!(RELEASED.load(std::sync::atomic::Ordering::SeqCst), 0);
    for (f, h) in c.take_releases() {
        unsafe { f(h) };
    }
    let r = RELEASED.load(std::sync::atomic::Ordering::SeqCst);
    assert!(r <= 10 && r >= 5, "dead extdata released ({r}), live one not");
    unsafe {
        assert!(live.has_flag(flags::EXTDATA));
        assert_eq!(*(live.std_data() as *const f64).add(1), 2.5);
    }
}

#[test]
fn layout_is_compact() {
    // docs/core.md §2: the layout is robigo's; robigo's headers map
    // GNU R's macros onto it
    let mut c = Core::new();
    let v = c.alloc_vector(INTSXP, 1);
    let n = c.alloc_node(LISTSXP, v, c.nil, c.nil);
    unsafe {
        assert_eq!(*v.0, INTSXP, "type: byte 0");
        assert_eq!(*(v.0.add(16) as *const isize), 1, "length at +16");
        assert_eq!(v.std_data() as usize - v.addr(), 32, "payload at +32");
        assert_eq!(*(n.0.add(16) as *const usize), v.addr(), "CAR at +16");
        assert_eq!(*(n.0.add(24) as *const usize), c.nil.addr(), "CDR at +24");
        assert_eq!(*(n.0.add(8) as *const usize), c.nil.addr(), "ATTRIB at +8");
        n.set_gp(0xABCD);
        assert_eq!(*(n.0.add(2) as *const u16), 0xABCD, "gp at +2");
        assert!(n.tracks_refs());
        let ch = c.mk_char(b"x", 0);
        assert_eq!(std::ffi::CStr::from_ptr(ch.0.add(32) as *const std::ffi::c_char).to_bytes(), b"x");
    }
}

#[test]
fn symbol_values_are_traced() {
    let mut c = Core::new();
    let s = c.install(b"some_symbol");
    let v = c.alloc_vector(REALSXP, 3);
    let addr = v.addr();
    unsafe { s.set_cdr_raw(v) };
    c.write_barrier(s);
    let _ = v;
    std::hint::black_box([0usize; 64]);
    for _ in 0..3 {
        c.collect();
    }
    for i in 0..100_000 {
        let x = c.alloc_vector(REALSXP, 3);
        unsafe { *(x.std_data() as *mut f64) = i as f64 };
    }
    unsafe { assert_eq!(s.cdr().addr(), addr) };
    assert!(c.resolve(addr).is_some());
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
    let _ = dead_key;
    std::hint::black_box([0usize; 64]);
    c.collect();
    let mut q = Vec::new();
    while let Some(w) = c.pop_finalizable() {
        q.push(w);
    }
    assert!(q.len() <= 1, "only the dead key's reference is ready");
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
    let _r = &mut reached as *mut bool;
    let ok = robigo_core::unwind::try_run(move || unsafe {
        robigo_core::unwind::jump();
        #[allow(unreachable_code)]
        {
            *_r = true;
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

/// Allocate a 4-double vector and its neighbour; return only the
/// one-past-the-end pointer of the first.
#[inline(never)]
fn alloc_with_end_pointer(c: &mut Core) -> usize {
    let x = c.alloc_vector(REALSXP, 4);
    unsafe {
        for i in 0..4 {
            *(x.std_data() as *mut f64).add(i) = 1.5;
        }
    }
    let n = c.alloc_vector(REALSXP, 4);
    assert_eq!(n.addr(), x.addr() + 64, "adjacent slots");
    std::hint::black_box(n);
    unsafe { x.std_data() as usize + 32 }
}

/// Overwrite the stack below the caller's frame (stale copies of objects).
#[inline(never)]
fn clobber_stack() {
    let a = [0usize; 4096];
    std::hint::black_box(&a);
}

#[test]
fn one_past_the_end_keeps_the_object() {
    let mut c = Core::new();
    // the two allocations must be adjacent: no collection between them
    c.set_torture(false);
    // 4 doubles: 32 + 32 = 64 bytes, a whole number of granules
    let end = alloc_with_end_pointer(&mut c);
    // hide the pointer's value in a form the scan cannot read, except
    // through the one-past-the-end word kept below
    let end_word = std::hint::black_box(end);
    clobber_stack();
    // poison freed slots so a wrongly freed object is visible
    c.set_torture(true);
    c.collect();
    c.collect();
    assert_eq!(unsafe { *((end_word - 8) as *const f64) }, 1.5, "only a one-past-the-end pointer referenced it");
    std::hint::black_box(end_word);
}

#[test]
fn medium_vectors_share_blocks() {
    // 10,000 live 1000-double vectors (8 KiB of payload each): before the
    // medium classes each took a 64 KiB chunk (655 MB reserved)
    let mut c = core_with_roots();
    let keep = c.alloc_vector(VECSXP, 10_000);
    // (under ROBIGO_GCTORTURE this collects on every allocation: it also
    // checks that collections do not abandon a block's unused space)
    root(keep);
    for i in 0..10_000 {
        let v = c.alloc_vector(REALSXP, 1000);
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
    }
    let reserved = c.heap_reserved();
    eprintln!("reserved for 10k x 8 KiB vectors: {} MB", reserved >> 20);
    assert!(reserved < 120 << 20, "{reserved}");
}

fn private_dirty_kb() -> usize {
    let s = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("Private_Dirty:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Lead requirement A.1 (docs/core.md §10): a forked child that collects
/// must not copy the parent's live heap. Prints the child's private dirty
/// memory after one collection over a ~200 MB live heap.
#[test]
fn collecting_in_a_forked_child_does_not_copy_the_heap() {
    let mut c = core_with_roots();
    // a measurement over a large live heap: no collection per allocation
    c.set_torture(false);
    let n = 400_000;
    let keep = c.alloc_vector(VECSXP, n);
    root(keep);
    for i in 0..n {
        // 512-byte payloads: ~200 MB live
        let v = c.alloc_vector(REALSXP, 64);
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
    }
    c.collect();
    let live_mb = c.heap_reserved() >> 20;
    let mut fds = [0i32; 2];
    unsafe { libc::pipe(fds.as_mut_ptr()) };
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let before = private_dirty_kb();
        c.collect();
        let after = private_dirty_kb();
        let d = (after.saturating_sub(before)) as u64;
        unsafe {
            libc::write(fds[1], &d as *const u64 as *const libc::c_void, 8);
            libc::_exit(0);
        }
    }
    let mut d = 0u64;
    unsafe {
        libc::read(fds[0], &mut d as *mut u64 as *mut libc::c_void, 8);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
    eprintln!("live heap {live_mb} MB; child's private dirty growth after one collection: {} MB", d >> 10);
    assert!(
        (d >> 10) * 4 < live_mb as u64,
        "a collection in a forked child copied {} MB of a {live_mb} MB heap",
        d >> 10
    );
}

#[test]
fn collections_do_not_abandon_bump_space() {
    let mut c = core_with_roots();
    let keep = c.alloc_vector(VECSXP, 2_000);
    root(keep);
    for i in 0..2_000 {
        let v = c.alloc_vector(REALSXP, 1000);
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
        // a collection after every allocation: each stranded the rest of
        // the current block before the fix (2,000 blocks)
        c.collect();
    }
    let blocks = c.heap_reserved() / (64 << 10);
    assert!(blocks < 400, "{blocks} blocks for 2,000 x 8 KiB live vectors (8 per block)");
}

thread_local! {
    static HOST_EDGES: RefCell<std::collections::HashMap<usize, Vec<Sexp>>> = RefCell::new(std::collections::HashMap::new());
}

unsafe fn trace_test_host(c: &mut Core, handle: usize) {
    HOST_EDGES.with(|edges| {
        if let Some(children) = edges.borrow().get(&handle) {
            for &child in children {
                c.mark(child);
            }
        }
    });
}

fn host_core() -> Box<Core> {
    HOST_EDGES.with(|edges| edges.borrow_mut().clear());
    let mut c = core_with_roots();
    c.set_scan_top(0);
    c.host_trace = Some(trace_test_host);
    c.gc_inhibit += 1;
    c
}

fn test_weak(c: &mut Core, key: Sexp, fin: Sexp) -> Sexp {
    let w = c.alloc_vector(WEAKREFSXP, 4);
    unsafe {
        let data = w.std_data() as *mut Sexp;
        *data = key;
        *data.add(2) = fin;
    }
    c.register_weak_ref(w);
    w
}

#[test]
fn host_trace_preserves_native_roots_and_terminates_on_cycles() {
    let mut c = host_core();
    let a = c.alloc_host(ENVSXP, 1, None);
    let b = c.alloc_host(ENVSXP, 2, None);
    HOST_EDGES.with(|e| {
        e.borrow_mut().insert(1, vec![b]);
        e.borrow_mut().insert(2, vec![a]);
    });
    root(a);
    c.gc_inhibit -= 1;
    c.collect();
    assert!(c.resolve(a.addr()).is_some());
    assert!(c.resolve(b.addr()).is_some());
    ROOTS.with(|r| r.borrow_mut().clear());
    c.collect();
    assert!(c.resolve(a.addr()).is_none());
    assert!(c.resolve(b.addr()).is_none());
}

#[test]
fn host_edges_participate_in_weak_reference_fixed_point() {
    let mut c = host_core();
    let key = c.alloc_host(ENVSXP, 1, None);
    let finalizer = c.alloc_host(CLOSXP, 2, None);
    let other_key = c.alloc_host(ENVSXP, 3, None);
    let other_fin = c.alloc_vector(RAWSXP, 8);
    HOST_EDGES.with(|e| e.borrow_mut().insert(2, vec![other_key]));
    let first = test_weak(&mut c, key, finalizer);
    let second = test_weak(&mut c, other_key, other_fin);
    root(key);
    c.gc_inhibit -= 1;
    c.collect();
    assert!(c.pop_finalizable().is_none());
    assert_eq!(c.weak_refs().len(), 2);
    ROOTS.with(|r| r.borrow_mut().clear());
    c.collect();
    assert_eq!(c.pop_finalizable(), Some(second));
    assert_eq!(c.pop_finalizable(), Some(first));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn manually_finalized_weak_reference_is_removed_even_with_nil_key() {
    let mut c = host_core();
    let nil = c.nil;
    let w = test_weak(&mut c, nil, nil);
    unsafe {
        w.set_gp(w.gp() | gp::READY_TO_FINALIZE);
    }
    c.gc_inhibit -= 1;
    c.collect();
    assert!(c.weak_refs().is_empty());
    assert_eq!(c.pop_finalizable(), Some(w));
    assert!(c.pop_finalizable().is_none());
}

/// GNU R's singly-linked weak list can lose a reference registered during a
/// finalizer batch when a later ready entry is removed at the head
/// (memory.c:RunFinalizers' `last == R_NilValue` branch overwrites
/// R_weak_refs). robigo keeps it and runs it on the next round: losing a
/// resource finalizer is not a behaviour worth reproducing.
#[test]
fn a_reference_registered_during_a_finalizer_batch_waits_for_the_next_round() {
    let mut c = host_core();
    let key = c.alloc_host(ENVSXP, 1, None);
    let fin = c.alloc_vector(RAWSXP, 8);
    let first = test_weak(&mut c, key, fin);
    c.gc_inhibit -= 1;
    c.collect();
    assert_eq!(c.pop_finalizable(), Some(first));
    // A finalizer registers another key while the batch is running.
    let later_key = c.alloc_host(ENVSXP, 2, None);
    root(later_key);
    let later = test_weak(&mut c, later_key, fin);
    assert_eq!(c.weak_refs(), &[later], "the new reference is not lost");
    // Its key dies: the next collection queues it.
    ROOTS.with(|r| r.borrow_mut().clear());
    c.collect();
    assert_eq!(c.pop_finalizable(), Some(later));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn all_finalizers_of_a_dead_key_are_queued_in_the_same_collection() {
    let mut c = host_core();
    let key = c.alloc_host(ENVSXP, 1, None);
    let fin = c.alloc_vector(RAWSXP, 8);
    let first = test_weak(&mut c, key, fin);
    let second = test_weak(&mut c, key, fin);
    c.gc_inhibit -= 1;
    c.collect();
    assert_eq!(c.pop_finalizable(), Some(second));
    assert_eq!(c.pop_finalizable(), Some(first));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn exit_finalizers_queue_in_weak_reference_list_order() {
    let mut c = host_core();
    let first_key = c.alloc_host(ENVSXP, 1, None);
    let second_key = c.alloc_host(ENVSXP, 2, None);
    let fin = c.alloc_vector(RAWSXP, 8);
    let first = test_weak(&mut c, first_key, fin);
    let second = test_weak(&mut c, second_key, fin);
    root(first_key);
    unsafe {
        second.set_gp(second.gp() | gp::FINALIZE_ON_EXIT);
    }
    unsafe { c.queue_exit_finalizers() };
    // only the onexit reference leaves the weak list; the other stays live
    assert_eq!(c.weak_refs(), &[first]);
    assert_eq!(c.pending_finalizer_count(), 1);
    assert_eq!(c.pop_finalizable(), Some(second));
    assert!(c.pop_finalizable().is_none());
    // the live key is not queued by a later collection
    c.gc_inhibit -= 1;
    c.collect();
    assert!(c.pop_finalizable().is_none());
    ROOTS.with(|r| r.borrow_mut().clear());
}
