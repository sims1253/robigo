//! robigo-heap's heap, strings, symbols and unwinding, tested directly.

// Tests poke objects directly: a safety comment on each block would say "this
// test built that object" a hundred times.
#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_heap::types::*;
use robigo_heap::{Core, HeapReport, Sexp, flags, gp};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};

thread_local! {
    static ROOTS: RefCell<Vec<Sexp>> = const { RefCell::new(Vec::new()) };
    /// Non-zero: what heap chunks allocated on this thread have at +8.
    static CHUNK_WORD: Cell<u64> = const { Cell::new(0) };
    /// Non-zero: the address the next heap chunk allocated on this thread
    /// is mapped at (Linux), whatever the allocator's history.
    static PLACE_CHUNK_AT: Cell<usize> = const { Cell::new(0) };
    /// The chunk mapped that way, (address, size), unmapped when freed.
    #[cfg(target_os = "linux")]
    static PLACED_CHUNK: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// The chunk requested by [`PLACE_CHUNK_AT`], mapped exactly there, or
/// null (the address is taken, or the system cannot place it).
#[cfg(target_os = "linux")]
fn map_placed_chunk(at: usize, size: usize) -> *mut u8 {
    // SAFETY: a new anonymous private mapping; MAP_FIXED_NOREPLACE fails
    // rather than replace anything mapped there.
    let p = unsafe {
        libc::mmap(
            at as *mut libc::c_void,
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        return std::ptr::null_mut();
    }
    if p as usize != at {
        // (a kernel before 4.17 takes the flag as a hint)
        // SAFETY: the mapping just made, unused.
        unsafe { libc::munmap(p, size) };
        return std::ptr::null_mut();
    }
    PLACED_CHUNK.set((at, size));
    p as *mut u8
}

#[cfg(not(target_os = "linux"))]
fn map_placed_chunk(_at: usize, _size: usize) -> *mut u8 {
    std::ptr::null_mut()
}

/// The system allocator, except that a heap chunk (the only 64 KiB-aligned
/// request) can be made to arrive with chosen contents, as recycled memory
/// does: `malloc` gives no guarantee, so a test cannot wait for a chunk to
/// come back dirty.
struct DirtyChunks;

// SAFETY: every request is forwarded to `System`, except a heap chunk that
// a test asked to place (`PLACE_CHUNK_AT`): a fresh mapping of the size
// asked, at a 64 KiB-aligned address, unmapped by `dealloc`. The other
// addition writes one word inside a block that `System` just returned for
// a layout of at least 64 KiB.
unsafe impl GlobalAlloc for DirtyChunks {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // only a heap chunk takes the placement; other allocations pass by
        let at = if layout.align() == 1 << 16 { PLACE_CHUNK_AT.replace(0) } else { 0 };
        if at != 0 && at.is_multiple_of(layout.align()) {
            let p = map_placed_chunk(at, layout.size());
            if !p.is_null() {
                return p;
            }
        }
        // SAFETY: the caller's contract, forwarded.
        let p = unsafe { System.alloc(layout) };
        if layout.align() == 1 << 16 && !p.is_null() {
            let word = CHUNK_WORD.with(Cell::get);
            if word != 0 {
                // SAFETY: a heap chunk is at least 64 KiB, and aligned for u64.
                unsafe { *(p as *mut u64).add(1) = word };
            }
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        #[cfg(target_os = "linux")]
        if PLACED_CHUNK.get() == (p as usize, layout.size()) {
            PLACED_CHUNK.set((0, 0));
            // SAFETY: the mapping `map_placed_chunk` made for this chunk.
            unsafe { libc::munmap(p as *mut libc::c_void, layout.size()) };
            return;
        }
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.realloc(p, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: DirtyChunks = DirtyChunks;

unsafe fn mark_roots(c: &mut Core) {
    let roots = ROOTS.with(|r| r.borrow().clone());
    for r in roots {
        unsafe {
            c.mark(r);
        }
    }
}

fn core_with_roots() -> Box<Core> {
    ROOTS.with(|r| r.borrow_mut().clear());
    let mut c = Core::new();
    unsafe { c.add_root_fn(mark_roots) };
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

/// # Safety
/// `x` must be a live INTSXP whose elements are initialized, and the
/// returned slice must not outlive `x` (collections free unreachable
/// objects) nor alias mutation of its payload.
unsafe fn ints<'a>(x: Sexp) -> &'a [i32] {
    unsafe { std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize) }
}

#[test]
fn constants() {
    let c = Core::new();
    unsafe {
        assert_eq!(c.nil().typ(), NILSXP);
        assert_eq!(c.nil().car(), c.nil());
        assert_eq!(c.nil().attrib_raw(), c.nil());
        assert!(c.nil().has_flag(flags::IMMORTAL));
        assert_eq!(c.na_string().char_bytes(), b"NA");
        assert_eq!(c.blank_string().char_bytes(), b"");
        assert_eq!(c.unbound().typ(), SYMSXP);
        assert_ne!(c.unbound(), c.missing_arg());
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
        assert!((0..3).all(|i| *sd.add(i) == c.blank_string()));
        let ld = l.std_data() as *const Sexp;
        assert!((0..2).all(|i| *ld.add(i) == c.nil()));
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
    assert_ne!(c.mk_char(b"NA", 0), c.na_string());
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
    let mut list = c.nil();
    for i in 0..1000 {
        let v = int_vec(&mut c, &[i]);
        list = unsafe { c.alloc_node(LISTSXP, v, list, c.nil()) };
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
        while p != c.nil() {
            assert_eq!(ints(p.car()), &[999 - n]);
            n += 1;
            p = p.cdr();
        }
        assert_eq!(n, 1000);
    }
    assert!(after < 10_000, "garbage freed: {after} live");
}

/// Allocation collects when GNU R's would: once the nodes in use reach
/// R_NSize (350,000), not before (memory.c NO_FREE_NODES).
#[test]
fn allocation_collects_when_the_node_count_reaches_r_nsize() {
    let mut c = core_with_roots();
    c.set_torture(false);
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert_eq!(n_size, 350_000);
    let n0 = c.stats.collections;
    for _ in in_use..n_size {
        unsafe { c.alloc_node(LISTSXP, c.nil(), c.nil(), c.nil()) };
    }
    assert_eq!(c.stats.collections, n0, "no collection before R_NSize nodes");
    unsafe { c.alloc_node(LISTSXP, c.nil(), c.nil(), c.nil()) };
    assert_eq!(c.stats.collections, n0 + 1);
    // the garbage is gone from the count, and the heap kept its size
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert_eq!(n_size, 350_000);
    assert!(in_use < 1_000, "{in_use} nodes in use after the collection");
}

/// The vector heap triggers on its own: R_VSize is 64 MB of 8-byte VECRECs,
/// and a vector that does not fit in what is free collects first
/// (allocVector3's `VHEAP_FREE() < alloc_size`).
#[test]
fn allocation_collects_when_the_vector_heap_is_full() {
    let mut c = core_with_roots();
    c.set_torture(false);
    let v_size = c.heap_sizes().v_size;
    assert_eq!(v_size, (67_108_864 + 1) / 8);
    let n0 = c.stats.collections;
    // seven 8 MB vectors fit in 64 MB (with what startup took)
    for _ in 0..7 {
        c.alloc_vector(REALSXP, 1 << 20);
    }
    assert_eq!(c.stats.collections, n0);
    let valloc = c.heap_sizes().vecrecs_in_use;
    assert!(valloc >= 7 << 20);
    c.alloc_vector(REALSXP, 1 << 20);
    assert_eq!(c.stats.collections, n0 + 1, "the eighth 8 MB vector collects first");
}

/// A live heap that outgrows R_NSize grows it (AdjustHeapSize) instead of
/// collecting on every allocation.
#[test]
fn a_growing_live_heap_grows_r_nsize() {
    let mut c = core_with_roots();
    c.set_torture(false);
    let mut list = c.nil();
    for i in 0..1_000_000 {
        list = unsafe { c.alloc_node(LISTSXP, c.nil(), list, c.nil()) };
        ROOTS.with(|r| {
            let mut r = r.borrow_mut();
            r.clear();
            r.push(list);
        });
        if i % 100_000 == 0 {
            assert!(c.stats.collections <= 10, "{} collections", c.stats.collections);
        }
    }
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert!(in_use >= 1_000_000);
    assert!(n_size > in_use, "R_NSize {n_size} grew past the live nodes {in_use}");
    assert!(c.stats.collections <= 10, "{} collections for 1M live nodes", c.stats.collections);
}

/// While collections are inhibited a due collection is postponed: the
/// heap sizes make room, and the first allocation afterwards collects
/// (R_gc_internal's `!R_GCEnabled` branch and gc_pending).
#[test]
fn an_inhibited_collection_runs_at_the_next_allocation() {
    let mut c = core_with_roots();
    c.set_torture(false);
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    let n0 = c.stats.collections;
    c.gc_inhibit += 1;
    for _ in in_use..n_size + 10 {
        unsafe { c.alloc_node(LISTSXP, c.nil(), c.nil(), c.nil()) };
    }
    assert_eq!(c.stats.collections, n0);
    let grown = c.heap_sizes().n_size;
    assert!(grown > n_size);
    c.gc_inhibit -= 1;
    unsafe { c.alloc_node(LISTSXP, c.nil(), c.nil(), c.nil()) };
    assert_eq!(c.stats.collections, n0 + 1);
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
    unsafe {
        c.preserve(x);
    }
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
    // SAFETY: `buf` outlives these objects and count_release never reads it.
    let live = unsafe { c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1000, Some(count_release)) };
    root(live);
    for _ in 0..10 {
        unsafe {
            c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1, Some(count_release));
        }
    }
    // overwrite stack slots that may still keep the dead objects
    clobber_stack();
    for _ in 0..3 {
        c.collect();
    }
    // release callbacks wait for a safe point
    assert_eq!(RELEASED.load(std::sync::atomic::Ordering::SeqCst), 0);
    for (f, h) in c.take_releases() {
        unsafe { f(h) };
    }
    let r = RELEASED.load(std::sync::atomic::Ordering::SeqCst);
    assert!((5..=10).contains(&r), "dead extdata released ({r}), live one not");
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
    let n = unsafe { c.alloc_node(LISTSXP, v, c.nil(), c.nil()) };
    unsafe {
        assert_eq!(*v.0, INTSXP, "type: byte 0");
        assert_eq!(*(v.0.add(16) as *const isize), 1, "length at +16");
        assert_eq!(v.std_data() as usize - v.addr(), 32, "payload at +32");
        assert_eq!(*(n.0.add(16) as *const usize), v.addr(), "CAR at +16");
        assert_eq!(*(n.0.add(24) as *const usize), c.nil().addr(), "CDR at +24");
        assert_eq!(*(n.0.add(8) as *const usize), c.nil().addr(), "ATTRIB at +8");
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
    unsafe { s.set_cdr(v) };
    let _ = v;
    clobber_stack();
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
            *d.add(1) = c.nil();
            *d.add(2) = fin;
        }
        unsafe {
            c.register_weak_ref(w);
        }
        w
    };
    let w1 = mk(&mut c, key_live);
    root(w1);
    let dead_key = c.alloc_vector(INTSXP, 1);
    let _w2 = mk(&mut c, dead_key);
    let dead_addr = dead_key.addr();
    let _ = dead_key;
    clobber_stack();
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
fn trampolines_and_jump() {
    use robigo_heap::unwind::{depth, jump, robigo_heap_try_void_ptr, robigo_heap_try_w6};
    use std::ffi::c_void;
    unsafe extern "C" fn nothing(_: *mut c_void) {}
    unsafe extern "C" fn jumps(reached: *mut c_void) {
        if std::hint::black_box(true) {
            // SAFETY: this frame contains nothing to drop.
            unsafe { jump() }
        }
        // SAFETY: `reached` is the test's bool (never reached).
        unsafe { *(reached as *mut bool) = true }
    }
    unsafe extern "C" fn nested(_: *mut c_void) {
        assert_eq!(depth(), 1);
        let mut reached = false;
        // SAFETY: `jumps` takes the bool's address.
        let inner = unsafe { robigo_heap_try_void_ptr(jumps, &mut reached as *mut bool as *mut c_void) };
        assert_eq!(inner, 1);
        assert!(!reached);
    }
    unsafe extern "C" fn add(a: usize, b: usize) -> usize {
        a + b
    }
    // SAFETY: the bodies are plain functions of one pointer argument.
    unsafe {
        assert_eq!(robigo_heap_try_void_ptr(nothing, std::ptr::null_mut()), 0);
        let mut reached = false;
        assert_eq!(robigo_heap_try_void_ptr(jumps, &mut reached as *mut bool as *mut c_void), 1);
        assert!(!reached);
        // nested: the inner context catches, the outer completes
        assert_eq!(robigo_heap_try_void_ptr(nested, std::ptr::null_mut()), 0);
        // two word arguments of six, a word back
        let mut out = 0usize;
        let a = [40usize, 2, 0, 0, 0, 0];
        assert_eq!(robigo_heap_try_w6(add as *const c_void, a.as_ptr(), &mut out), 0);
        assert_eq!(out, 42);
    }
    assert_eq!(depth(), 0);
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

/// As [`alloc_with_end_pointer`], then shorten the vector in place
/// (`SETLENGTH`, optionally growable as `SET_GROWABLE_BIT` makes it): C
/// keeps the end of the buffer it allocated, not of the length.
#[inline(never)]
fn alloc_shortened_with_end_pointer(c: &mut Core, growable: bool) -> usize {
    let x = c.alloc_vector(REALSXP, 4);
    unsafe {
        for i in 0..4 {
            *(x.std_data() as *mut f64).add(i) = 1.5;
        }
        x.set_truelen(4);
        if growable {
            x.set_gp(x.gp() | gp::GROWABLE);
        }
        x.set_std_len(2);
    }
    let n = c.alloc_vector(REALSXP, 4);
    assert_eq!(n.addr(), x.addr() + 64, "adjacent slots");
    std::hint::black_box(n);
    unsafe { x.std_data() as usize + 32 }
}

#[test]
fn the_end_of_a_shortened_vector_keeps_it() {
    for growable in [false, true] {
        let mut c = Core::new();
        c.set_torture(false);
        let end = alloc_shortened_with_end_pointer(&mut c, growable);
        let end_word = std::hint::black_box(end);
        clobber_stack();
        c.set_torture(true);
        c.collect();
        c.collect();
        assert!(c.resolve(end_word - 64).is_some(), "growable {growable}: only the buffer's end pointer referenced it");
        assert_eq!(unsafe { *((end_word - 8) as *const f64) }, 1.5, "growable {growable}");
        std::hint::black_box(end_word);
    }
}

/// A shortened large vector whose data ends exactly at its chunk's end
/// (`DATA_OFF + OFF_DATA + 16,246 * 8 = 131,072`); its end pointer.
#[inline(never)]
fn alloc_large_ending_at_its_chunk(c: &mut Core) -> usize {
    let x = c.alloc_vector(REALSXP, 16_246);
    unsafe {
        *(x.std_data() as *mut f64).add(16_245) = 1.5;
        x.set_truelen(16_246);
        x.set_std_len(2);
        x.std_data() as usize + 16_246 * 8
    }
}

#[test]
fn the_end_of_the_highest_chunk_keeps_its_vector() {
    if !cfg!(all(target_os = "linux", target_pointer_width = "64")) {
        eprintln!(
            "skipped: placing a heap chunk above the others needs Linux's MAP_FIXED_NOREPLACE and a 64-bit address space"
        );
        return;
    }
    let mut c = Core::new();
    c.set_torture(false);
    // the vector's chunk above every other chunk of this heap, however
    // earlier tests left the allocator: mapped at a free address 1 GiB
    // and more above the heap's top
    let mut end = 0;
    for k in 1..=16 {
        PLACE_CHUNK_AT.set((c.heap_bounds().1 + (k << 30)) & !0xffff);
        end = alloc_large_ending_at_its_chunk(&mut c);
        PLACE_CHUNK_AT.set(0);
        if end == c.heap_bounds().1 {
            break;
        }
    }
    assert_eq!(end % 65536, 0);
    assert_eq!(end, c.heap_bounds().1, "precondition: a vector's data ends at the heap's highest address");
    let end_word = std::hint::black_box(end);
    clobber_stack();
    c.collect();
    assert!(c.resolve(end_word - 1).is_some(), "only the end pointer at the heap's top referenced it");
    assert_eq!(unsafe { *((end_word - 8) as *const f64) }, 1.5);
    std::hint::black_box(end_word);
}

/// Allocate an external pointer and its neighbour; return the neighbour
/// (whose address is one past the pointer's slot) and the pointer's
/// address in a form the scan cannot read.
#[inline(never)]
fn alloc_with_neighbour(c: &mut Core) -> (Sexp, usize) {
    let nil = c.nil();
    let x = unsafe { c.alloc_node(EXTPTRSXP, Sexp::NULL, nil, nil) };
    let n = unsafe { c.alloc_node(LISTSXP, nil, nil, nil) };
    assert_eq!(n.addr(), x.addr() + robigo_heap::NODE_SIZE, "adjacent slots");
    (n, !x.addr())
}

#[test]
fn a_pointer_to_the_next_object_does_not_keep_a_node() {
    let mut c = Core::new();
    // the two allocations must be adjacent: no collection between them
    c.set_torture(false);
    let (n, hidden) = alloc_with_neighbour(&mut c);
    // the stack contains `n`, the word one past the pointer's slot: a node
    // has no data a C loop runs to the end of, so the word is `n`'s alone
    let n = std::hint::black_box(n);
    clobber_stack();
    c.collect();
    assert!(c.resolve(n.addr()).is_some(), "the neighbour is referenced");
    assert!(c.resolve(!hidden).is_none(), "the external pointer is garbage");
    std::hint::black_box(n);
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
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: Private_Dirty needs Linux's smaps_rollup");
        return;
    }
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
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe failed");
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
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
    // drop the write end, or read() below never sees EOF if the child dies
    unsafe { libc::close(fds[1]) };
    let mut buf = [0u8; 8];
    let mut got = 0;
    while got < buf.len() {
        // a child that hangs after fork (e.g. on a lock another test thread
        // held) fails the test instead of hanging the run
        let mut pfd = libc::pollfd { fd: fds[0], events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut pfd, 1, 120_000) } != 1 {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
            panic!("the forked child hung after fork");
        }
        let n = unsafe { libc::read(fds[0], buf[got..].as_mut_ptr().cast(), buf.len() - got) };
        assert!(n > 0, "the child died before reporting its measurement");
        got += n as usize;
    }
    let d = u64::from_ne_bytes(buf);
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "the forked child failed");
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

/// A core whose collections see only the precise roots: no stack word
/// keeps a key alive.
fn weak_core() -> Box<Core> {
    let mut c = core_with_roots();
    c.set_scan_top(0);
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
    unsafe {
        c.register_weak_ref(w);
    }
    w
}

#[test]
fn finalizers_participate_in_the_weak_reference_fixed_point() {
    let mut c = weak_core();
    let key = c.alloc_vector(INTSXP, 1);
    let finalizer = c.alloc_vector(VECSXP, 1);
    let other_key = c.alloc_vector(INTSXP, 1);
    let other_fin = c.alloc_vector(RAWSXP, 8);
    // only the first reference's finalizer reaches the second one's key
    unsafe { *(finalizer.std_data() as *mut Sexp) = other_key };
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
    let mut c = weak_core();
    let nil = c.nil();
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
    let mut c = weak_core();
    let key = c.alloc_vector(INTSXP, 1);
    let fin = c.alloc_vector(RAWSXP, 8);
    let first = test_weak(&mut c, key, fin);
    c.gc_inhibit -= 1;
    c.collect();
    assert_eq!(c.pop_finalizable(), Some(first));
    // A finalizer registers another key while the batch is running.
    let later_key = c.alloc_vector(INTSXP, 1);
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
    let mut c = weak_core();
    let key = c.alloc_vector(INTSXP, 1);
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
    let mut c = weak_core();
    let first_key = c.alloc_vector(INTSXP, 1);
    let second_key = c.alloc_vector(INTSXP, 1);
    let fin = c.alloc_vector(RAWSXP, 8);
    let first = test_weak(&mut c, first_key, fin);
    let second = test_weak(&mut c, second_key, fin);
    root(first_key);
    unsafe {
        second.set_gp(second.gp() | gp::FINALIZE_ON_EXIT);
    }
    c.queue_exit_finalizers();
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

/// A large object's mark word lives in its chunk's header. The chunk comes
/// from `malloc`, so if the word started out containing the next collection's
/// epoch the object would look marked already and its children would not
/// be traced.
#[test]
fn a_large_object_starts_unmarked_whatever_its_chunk_held() {
    let mut c = core_with_roots();
    c.set_torture(false);
    // only the rooted vector can keep the child alive
    c.set_scan_top(0);
    let next_epoch = c.stats.collections + 1;
    CHUNK_WORD.with(|w| w.set(next_epoch));
    let big = c.alloc_vector(VECSXP, 100_000);
    CHUNK_WORD.with(|w| w.set(0));
    root(big);
    let child = c.alloc_vector(INTSXP, 1);
    unsafe { *(big.std_data() as *mut Sexp) = child };
    c.collect();
    assert_eq!(c.stats.collections, next_epoch);
    assert!(c.resolve(child.addr()).is_some(), "the large vector's child was swept");
}

#[test]
#[should_panic(expected = "not a vector type")]
fn alloc_vector_rejects_a_non_vector_type() {
    Core::new().alloc_vector(LISTSXP, 5);
}

#[test]
#[should_panic(expected = "not a vector type")]
fn try_alloc_vector_rejects_a_non_vector_type() {
    Core::new().try_alloc_vector(ENVSXP, 1000);
}

/// A C++ `throw` is `_Unwind_RaiseException`, and `__cxa_throw` calls
/// `std::terminate` when that returns. It returns end-of-stack only if phase
/// 1 stops at the shim; otherwise it finds a handler above the shim
/// (libtest's `catch_unwind` here, a package's `catch` in real life) and
/// phase 2 takes the exception through Rust frames.
#[cfg(target_os = "linux")]
#[test]
fn a_foreign_exception_stops_at_the_c_shim() {
    use std::ffi::{c_int, c_void};
    const URC_END_OF_STACK: c_int = 5;

    #[repr(C, align(16))]
    struct UnwindException {
        class: u64,
        cleanup: usize,
        private: [u64; 6],
    }
    unsafe extern "C-unwind" {
        fn _Unwind_RaiseException(e: *mut UnwindException) -> c_int;
    }
    /// A package callback that throws: reports what the raise returned.
    unsafe extern "C-unwind" fn throw(result: *mut c_int) {
        let mut e = UnwindException { class: u64::from_be_bytes(*b"robigoT\0"), cleanup: 0, private: [0; 6] };
        // SAFETY: `e` is a well-formed exception object that outlives the
        // call, and `result` is the caller's local.
        unsafe { *result = _Unwind_RaiseException(&mut e) };
    }

    let mut raised: c_int = -1;
    let mut args = [std::ptr::null_mut::<c_void>(); 65];
    args[0] = (&raw mut raised).cast();
    let mut out = std::ptr::null_mut();
    let thrower = throw as unsafe extern "C-unwind" fn(*mut c_int);
    // SAFETY: `throw` reads one argument of the 65 and returns nothing, so
    // `out` receives a meaningless word that is never read.
    let unwound = unsafe {
        robigo_heap::unwind::robigo_heap_try_call65(
            thrower as *const c_void,
            args.as_mut_ptr(),
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(unwound, 0, "the callback returned");
    assert_eq!(raised, URC_END_OF_STACK);
}

/// Free slots have no start bit, so a sweep that walks the bitmap cannot
/// see the ones it left on the free list earlier.
#[test]
fn a_sweep_keeps_the_free_slots_of_earlier_sweeps() {
    // 32 + 124 * 8 = 1024 bytes: 62 slots to a block
    const LEN: usize = 124;
    const SLOTS: usize = 62;
    let mut c = core_with_roots();
    c.set_torture(false);
    c.set_scan_top(0);
    let keep = c.alloc_vector(VECSXP, SLOTS);
    root(keep);
    let slots = unsafe { keep.std_data() as *mut Sexp };
    for i in 0..SLOTS {
        let v = c.alloc_vector(REALSXP, LEN);
        unsafe { *slots.add(i) = v };
    }
    let mut freed = std::collections::HashSet::new();
    let mut drop_slot = |c: &mut Core, i: usize| unsafe {
        freed.insert((*slots.add(i)).addr());
        *slots.add(i) = c.nil();
    };
    let block = |x: usize| x & !0xFFFF;
    assert!((0..SLOTS).all(|i| unsafe { block((*slots.add(i)).addr()) == block((*slots).addr()) }));
    for i in (0..SLOTS).step_by(2) {
        drop_slot(&mut c, i);
    }
    c.collect();
    // one more death: this sweep walks the block
    drop_slot(&mut c, 1);
    c.collect();
    for _ in 0..freed.len() {
        let x = c.alloc_vector(REALSXP, LEN);
        assert!(freed.remove(&x.addr()), "{} free slots were lost", freed.len());
    }
}

unsafe fn collecting_root(c: &mut Core) {
    c.collect();
}

unsafe fn allocating_root(c: &mut Core) {
    c.alloc_vector(INTSXP, 1);
}

#[test]
#[should_panic(expected = "inside a collection")]
fn a_root_callback_cannot_collect() {
    let mut c = Core::new();
    unsafe { c.add_root_fn(collecting_root) };
    c.collect();
}

/// What ROBIGO_GCTORTURE finds in CI: an allocating hook nests a collection
/// as soon as allocation collects.
#[test]
#[should_panic(expected = "inside a collection")]
fn a_root_callback_cannot_allocate() {
    let mut c = Core::new();
    unsafe { c.add_root_fn(allocating_root) };
    c.set_torture(true);
    c.collect();
}

/// An immortal CHARSXP is counted once by `immortal_chars`; a store into it
/// must not count it again as marked, or the sum can equal the table's size
/// with a dead entry in it, and the prune is skipped.
#[test]
fn a_store_into_an_immortal_char_does_not_hide_dead_cache_entries() {
    let mut c = Core::new();
    c.set_torture(false);
    c.set_scan_top(0);
    let blank = c.blank_string();
    // the barrier fires on the header alone; the slot is a local, as no
    // CHARSXP field is a traced one
    let mut slot = c.nil();
    unsafe { blank.store(&mut slot, c.nil()) };
    let before = c.char_cache_len();
    c.mk_char(b"soon dead", 0);
    assert_eq!(c.char_cache_len(), before + 1);
    c.collect();
    assert_eq!(c.char_cache_len(), before, "a dead CHARSXP stayed in the cache");
}

/// gctorture2(step, wait): the first forced collection after `wait`
/// allocations, then one every `step` (memory.c FORCE_GC).
#[test]
fn gctorture_counts_allocations() {
    let mut c = Core::new();
    let collections = |c: &Core| c.stats.collections;
    let allocs = |c: &mut Core, n: usize| {
        for _ in 0..n {
            c.alloc_vector(INTSXP, 1);
        }
    };
    // step 3, wait 5
    c.gc_torture(3, 5);
    assert_eq!(c.torture_counters(), (5, 3));
    assert!(c.torture());
    let n0 = collections(&c);
    allocs(&mut c, 4);
    assert_eq!(collections(&c), n0);
    allocs(&mut c, 1);
    assert_eq!(collections(&c), n0 + 1);
    allocs(&mut c, 2);
    assert_eq!(collections(&c), n0 + 1);
    allocs(&mut c, 1);
    assert_eq!(collections(&c), n0 + 2);
    allocs(&mut c, 6);
    assert_eq!(collections(&c), n0 + 4);
    // NA and negative steps change nothing; a wait without a step is ignored
    c.gc_torture(i32::MIN, 7);
    c.gc_torture(-1, 7);
    assert_eq!(c.torture_counters().1, 3);
    c.gc_torture(0, 7);
    assert_eq!(c.torture_counters(), (0, 0));
    assert!(!c.torture());
    let n1 = collections(&c);
    allocs(&mut c, 100);
    assert_eq!(collections(&c), n1);
    // gctorture(TRUE): every allocation
    c.set_torture(true);
    allocs(&mut c, 10);
    assert_eq!(collections(&c), n1 + 10);
    // large vectors (past MAX_MEDIUM) take their own path and count too
    c.gc_torture(3, 5);
    let n2 = collections(&c);
    for _ in 0..5 {
        c.alloc_vector(REALSXP, 10_000);
    }
    assert_eq!(collections(&c), n2 + 1);
    assert_eq!(c.torture_counters(), (3, 3));
    c.set_torture(false);
}

/// Nodes are 48 bytes: the 4th word at +40 starts as R_NilValue, is
/// traced, and environments keep ENCLOS at +24 (council item 20).
#[test]
fn nodes_have_a_traced_fourth_word() {
    let mut c = core_with_roots();
    c.set_torture(false);
    c.set_scan_top(0);
    assert_eq!(robigo_heap::NODE_SIZE, 48);
    let n = unsafe { c.alloc_node(LISTSXP, c.nil(), c.nil(), c.nil()) };
    root(n);
    unsafe {
        assert_eq!(n.aux(), c.nil());
        assert_eq!(*(n.0.add(robigo_heap::OFF_AUX) as *const usize), c.nil().addr(), "aux at +40");
        assert_eq!(c.nil().aux(), c.nil(), "R_NilValue's 4th word is itself");
        assert_eq!(c.unbound().aux(), c.nil(), "a symbol's 4th word is nil");
    }
    let v = c.alloc_vector(REALSXP, 3);
    let addr = v.addr();
    unsafe { n.set_aux(v) };
    unsafe { assert_eq!(v.refcnt(), 1, "the store counts the reference") };
    for _ in 0..3 {
        c.collect();
    }
    for _ in 0..10_000 {
        c.alloc_vector(REALSXP, 3);
    }
    unsafe { assert_eq!(n.aux().addr(), addr, "the 4th word keeps its object alive") };

    let parent = unsafe { c.alloc_node(ENVSXP, c.nil(), c.nil(), c.nil()) };
    let e = unsafe { c.alloc_node(ENVSXP, c.nil(), c.nil(), c.nil()) };
    unsafe {
        robigo_heap::env::set_enclos(e, parent);
        assert_eq!(*(e.0.add(robigo_heap::OFF_ENCLOS) as *const usize), parent.addr(), "ENCLOS at +24");
        assert_eq!(robigo_heap::env::enclos(e), parent);
        assert_eq!(robigo_heap::env::frame(e), c.nil());
        assert_eq!(robigo_heap::env::hashtab(e), c.nil());
    }
}

/// The store barrier logs an immortal object once (its BARRIER bit goes)
/// and keeps what was stored alive, for a small node and for an immortal
/// large vector, whose chunk header carries the remembered set too.
#[test]
fn the_store_barrier_logs_immortals_once() {
    let mut c = Core::new();
    c.set_torture(false);
    c.set_scan_top(0);
    let s = c.install(b"barrier_symbol");
    unsafe {
        assert!(s.has_flag(flags::BARRIER), "an immortal waits for its first store");
    }
    let big = c.alloc_vector(VECSXP, 100_000);
    unsafe { c.make_immortal(big) };
    let fresh = c.alloc_vector(INTSXP, 2);
    unsafe {
        assert!(!fresh.has_flag(flags::BARRIER), "a mortal object takes the fast path");
    }
    let v1 = c.alloc_vector(REALSXP, 1);
    let v2 = c.alloc_vector(REALSXP, 2);
    let (a1, a2) = (v1.addr(), v2.addr());
    unsafe {
        s.set_cdr(v1);
        assert!(!s.has_flag(flags::BARRIER), "logged");
        big.set_elt(99_999, v2);
        assert!(!big.has_flag(flags::BARRIER), "logged");
        // a second store needs no second entry
        s.set_cdr(v1);
    }
    for _ in 0..3 {
        c.collect();
    }
    for _ in 0..10_000 {
        c.alloc_vector(REALSXP, 2);
    }
    unsafe {
        assert_eq!(s.cdr().addr(), a1);
        assert_eq!((*(big.std_data() as *const Sexp).add(99_999)).addr(), a2);
        assert!(c.resolve(a1).is_some() && c.resolve(a2).is_some());
    }
}

thread_local! {
    /// The core `robigo_heap_test_stack_params`'s hooks allocate in and collect.
    static ROUTINE_CORE: Cell<*mut Core> = const { Cell::new(std::ptr::null_mut()) };
}

unsafe extern "C" {
    fn robigo_heap_test_hooks(alloc: extern "C" fn() -> *mut std::ffi::c_void, collect: extern "C" fn());
    static robigo_heap_test_hidden: [usize; 2];
    // Address only: the C definition takes 65 arguments and returns SX and
    // is invoked through `robigo_heap_try_call65`; never call this directly.
    fn robigo_heap_test_stack_params();
}

extern "C" fn routine_alloc() -> *mut std::ffi::c_void {
    unsafe { (*ROUTINE_CORE.get()).alloc_vector(INTSXP, 1).0 as *mut std::ffi::c_void }
}

extern "C" fn routine_collect() {
    unsafe { (*ROUTINE_CORE.get()).collect() };
}

/// A routine's stack-passed parameters are C locals: a fresh object kept
/// only in its 7th and 65th parameter survives a collection, while the
/// trampoline narrows the scan to the routine's frames (`shim.c`). The
/// first narrowing, from a sampled stack pointer, left them out with Clang.
#[test]
fn stack_parameters_of_a_routine_are_roots() {
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    ROUTINE_CORE.set(core);
    unsafe {
        robigo_heap_test_hooks(routine_alloc, routine_collect);
        // as the bridge: an open segment of the scan from here down
        let m = 0usize;
        let here = std::hint::black_box(&m) as *const usize as usize;
        (*core).set_scan_top(here);
        let mut args = [std::ptr::null_mut::<std::ffi::c_void>(); 65];
        let mut out = std::ptr::null_mut();
        let t = robigo_heap::unwind::robigo_heap_try_call65(
            robigo_heap_test_stack_params as *const std::ffi::c_void,
            args.as_mut_ptr(),
            &mut out,
            (*core).scan_top_slot(),
        );
        assert_eq!(t, 0);
        assert_eq!((*core).scan_top(), here, "the limit is restored");
        (*core).set_scan_top(usize::MAX);
        let hidden = std::ptr::read_volatile(&raw const robigo_heap_test_hidden);
        assert!((*core).resolve(!hidden[0]).is_some(), "the 7th parameter's object was freed");
        assert!((*core).resolve(!hidden[1]).is_some(), "the 65th parameter's object was freed");
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}
