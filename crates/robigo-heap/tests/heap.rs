//! robigo-heap's heap, strings, symbols, collection protocol and unwinding,
//! tested directly.

// Tests poke objects directly: a safety comment on each block would say "this
// test built that object" a hundred times.
#![allow(clippy::undocumented_unsafe_blocks)]

use robigo_heap::types::*;
use robigo_heap::{
    AllocError, Core, GC_DUE, HeapReport, LARGE, NeedGc, Safepoint, ScanSegment, Sexp, StatsMode, Trace, Visitor,
    flags, gp,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

thread_local! {
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

// ---- helpers ---------------------------------------------------------------

/// A collection with `roots` (a `Vec<Sexp>`, `()`, ...): every object a
/// test still needs is in them, or immortal, precious, or held by a C
/// segment.
fn gc<R: Trace + ?Sized>(c: &mut Core, roots: &mut R) -> u64 {
    unsafe { c.collect(roots) }
}

/// A safepoint after an allocation, as a builtin's return makes one.
fn safepoint<R: Trace + ?Sized>(c: &mut Core, roots: &mut R) -> bool {
    unsafe { c.safepoint(Safepoint::Return, roots) }
}

fn int_vec(c: &mut Core, vals: &[i32]) -> Sexp {
    let x = c.alloc_vector(INTSXP, vals.len()).unwrap();
    unsafe {
        std::ptr::copy_nonoverlapping(vals.as_ptr(), x.std_data() as *mut i32, vals.len());
    }
    x
}

fn node(c: &mut Core, car: Sexp, cdr: Sexp) -> Sexp {
    let nil = c.nil();
    unsafe { c.alloc_node(LISTSXP, car, cdr, nil) }.unwrap()
}

/// # Safety
/// `x` must be a live INTSXP whose elements are initialized, and the
/// returned slice must not outlive `x` (collections free unreachable
/// objects) nor alias mutation of its payload.
unsafe fn ints<'a>(x: Sexp) -> &'a [i32] {
    unsafe { std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize) }
}

/// Live objects of a fresh heap after a collection: R's constants.
fn baseline_live() -> u64 {
    let mut c = Core::new();
    gc(&mut c, &mut ());
    c.stats.live_objects
}

// ---- native calls: C segments ----------------------------------------------

thread_local! {
    /// The core the test routines' hooks allocate in and collect.
    static ROUTINE_CORE: Cell<*mut Core> = const { Cell::new(std::ptr::null_mut()) };
    static ROUTINE_SP: Cell<usize> = const { Cell::new(0) };
}

unsafe extern "C" {
    fn robigo_heap_test_hooks(alloc: extern "C" fn() -> *mut c_void, collect: extern "C" fn());
    static robigo_heap_test_hidden: [usize; 2];
    // Address only: the C definitions take 65 arguments and return SX and
    // are invoked through `robigo_heap_try_call65`; never call these directly.
    fn robigo_heap_test_stack_params();
    fn robigo_heap_test_collect65();
    // (one argument, returns SX; called through `robigo_heap_try_call6`)
    fn robigo_heap_test_local6();
}

extern "C" fn routine_alloc() -> *mut c_void {
    unsafe { (*ROUTINE_CORE.get()).alloc_vector(INTSXP, 1).unwrap().0 as *mut c_void }
}

/// The stand-in for an export that collects (`R_gc`, an allocating one),
/// with the machine entry an export needs for an exact segment (x86-64):
/// push the callee-saved registers C left in place just below the return
/// address, and run the export's Rust body below them with the lowest
/// one's address, where the body closes its caller's C segment. No word of
/// the body's frame is in the segment.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
extern "C" fn routine_collect() {
    std::arch::naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // the lowest pushed register: the segment's low end
        "mov rdi, rsp",
        // (16-byte alignment at the call)
        "sub rsp, 8",
        "call {body}",
        "add rsp, 8",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
        body = sym collect_body,
    )
}

#[cfg(target_arch = "x86_64")]
extern "C" fn collect_body(lo: usize) {
    collect_closing(|seg| (seg, lo));
}

/// Elsewhere: the registers are spilled into the node, whose address is the
/// segment's low end (the body's frame above it is scanned too).
#[cfg(not(target_arch = "x86_64"))]
extern "C" fn routine_collect() {
    collect_closing(|seg| {
        seg.spill_registers();
        let lo = seg as *const ScanSegment as usize;
        (seg, lo)
    });
}

thread_local! {
    /// Non-zero: a word the export's body writes all over its own frame
    /// before it collects (a dead object's address, in
    /// `export_frames_keep_nothing`).
    static PLANT: Cell<usize> = const { Cell::new(0) };
}

/// The export's body: words of a Rust frame (`PLANT` everywhere in it),
/// then the segment closed at the low end `close` gives, a collection with
/// no Rust roots, and the segment reopened.
#[inline(always)]
fn collect_closing(close: impl FnOnce(&mut ScanSegment) -> (&mut ScanSegment, usize)) {
    let words = [PLANT.get(); 512];
    std::hint::black_box(&words);
    let mut node = ScanSegment::new();
    let (seg, lo) = close(&mut node);
    unsafe {
        let c = ROUTINE_CORE.get();
        (*c).close_scan_segment(seg, lo);
        (*c).collect(&mut ());
        (*c).reopen_scan_segment(seg);
    }
    std::hint::black_box(&words);
}

/// `f`, a routine of up to 65 arguments, called as a native entry: through
/// the trampoline with the core's segment slot, so the C segment opens at
/// its stack arguments. `args` fills the last slots (stack-passed on
/// x86-64 and AArch64). Returns where the caller's frame is.
#[inline(never)]
unsafe fn native_call(core: *mut Core, f: *const c_void, args: &[usize]) -> usize {
    unsafe {
        ROUTINE_CORE.set(core);
        robigo_heap_test_hooks(routine_alloc, routine_collect);
        let m = 0usize;
        let here = std::hint::black_box(&m) as *const usize as usize;
        let mut a = [std::ptr::null_mut::<c_void>(); 65];
        for (i, &w) in args.iter().enumerate() {
            a[65 - args.len() + i] = w as *mut c_void;
        }
        let mut out = std::ptr::null_mut();
        let mut jb = robigo_heap::unwind::JmpBuf::UNSET;
        let plain = (*core).call65_plain() as std::ffi::c_int;
        let t = robigo_heap::unwind::robigo_heap_try_call65(
            &mut jb,
            f,
            a.as_mut_ptr(),
            &mut out,
            (*core).scan_top_slot(),
            plain,
        );
        assert_eq!(t, 0);
        assert_eq!((*core).scan_top(), 0, "the native entry closed its segment as it returned");
        here
    }
}

/// `f`, a callback of up to six word arguments (an ALTREP method, a cend),
/// called as a native entry through the six-argument trampoline with the
/// core's segment slot. Returns where the caller's frame is.
#[inline(never)]
unsafe fn native_call6(core: *mut Core, f: *const c_void, a: [usize; 6]) -> usize {
    use robigo_heap::unwind::{JmpBuf, Ret6, robigo_heap_try_call6};
    unsafe {
        ROUTINE_CORE.set(core);
        robigo_heap_test_hooks(routine_alloc, routine_collect);
        let m = 0usize;
        let here = std::hint::black_box(&m) as *const usize as usize;
        let mut jb = JmpBuf::UNSET;
        let mut out = 0usize;
        let t =
            robigo_heap_try_call6(&mut jb, f, a.as_ptr(), Ret6::Word, (&raw mut out).cast(), (*core).scan_top_slot());
        assert_eq!(t, 0);
        assert_eq!((*core).scan_top(), 0, "the native entry closed its segment as it returned");
        here
    }
}

/// Collect inside a native routine whose stack arguments are `words`: the
/// only place the conservative scan sees them.
fn collect_holding(c: &mut Core, words: &[usize]) {
    let core: *mut Core = c;
    unsafe { native_call(core, robigo_heap_test_collect65 as *const c_void, words) };
}

// ---- objects ----------------------------------------------------------------

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
    let s = c.alloc_vector(STRSXP, 3).unwrap();
    let l = c.alloc_vector(VECSXP, 2).unwrap();
    let r = c.alloc_vector(REALSXP, 1000).unwrap();
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
    let a = c.mk_char(b"hello", 0).unwrap();
    let b = c.mk_char(b"hello", gp::UTF8).unwrap();
    assert_eq!(a, b, "ASCII drops the encoding bits");
    unsafe {
        assert!(a.gp() & gp::ASCII != 0);
        assert_eq!(*a.std_data().add(5), 0, "NUL-terminated");
    }
    let u = c.mk_char("café".as_bytes(), gp::UTF8).unwrap();
    let l = c.mk_char("café".as_bytes(), gp::LATIN1).unwrap();
    assert_ne!(u, l);
    assert_eq!(u, c.mk_char("café".as_bytes(), gp::UTF8).unwrap());
    assert_ne!(c.mk_char(b"NA", 0).unwrap(), c.na_string());
}

#[test]
fn symbols_are_unique_and_immortal() {
    let mut c = Core::new();
    let x = c.install(b"x").unwrap();
    assert_eq!(x, c.install(b"x").unwrap());
    assert_ne!(x, c.install(b"y").unwrap());
    unsafe {
        assert_eq!(x.typ(), SYMSXP);
        assert_eq!(x.car().char_bytes(), b"x");
        assert!(x.has_flag(flags::IMMORTAL));
    }
    for _ in 0..3 {
        gc(&mut c, &mut ());
    }
    unsafe { assert_eq!(c.install(b"x").unwrap().car().char_bytes(), b"x") };
}

/// Only the roots keep objects: no Rust frame is scanned, so what a
/// collection keeps is exact.
#[test]
fn rooted_objects_survive_and_garbage_is_freed() {
    let base = baseline_live();
    let mut c = Core::new();
    let keep = int_vec(&mut c, &[1, 2, 3]);
    let mut list = c.nil();
    for i in 0..1000 {
        let v = int_vec(&mut c, &[i]);
        list = node(&mut c, v, list);
    }
    // garbage
    for i in 0..100_000 {
        int_vec(&mut c, &[i, i]);
    }
    let mut roots = vec![keep, list];
    gc(&mut c, &mut roots);
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
    assert_eq!(c.stats.live_objects - base, 2001, "exactly the rooted objects are live");
}

/// The roots are any `Trace` value: a derived struct, an enum, a tuple of
/// them.
#[test]
fn derived_roots_keep_their_fields() {
    #[derive(Trace)]
    struct Frame {
        values: Vec<Sexp>,
        current: Option<Sexp>,
        depth: u32,
        #[trace(skip = "an address into `values`, which is traced")]
        _top: *const Sexp,
    }
    #[derive(Trace)]
    enum Pending {
        None,
        Value(Sexp),
        Pair { a: Sexp, b: Sexp },
    }
    #[derive(Trace)]
    struct Pair<T>(T, #[trace(skip = "a count, not an object")] usize);

    let base = baseline_live();
    let mut c = Core::new();
    let v = [int_vec(&mut c, &[1]), int_vec(&mut c, &[2]), int_vec(&mut c, &[3]), int_vec(&mut c, &[4])];
    let garbage = int_vec(&mut c, &[5]).addr();
    let mut frame = Frame { values: vec![v[0]], current: Some(v[1]), depth: 1, _top: std::ptr::null() };
    let mut pending = [Pending::Value(v[2]), Pending::Pair { a: v[3], b: c.nil() }, Pending::None];
    let mut pair = Pair(std::cell::Cell::new(c.nil()), 7);
    gc(&mut c, &mut (&mut frame, &mut pending, &mut pair));
    assert_eq!(c.stats.live_objects - base, 4);
    for x in v {
        assert!(c.resolve(x.addr()).is_some());
    }
    assert!(c.resolve(garbage).is_none(), "the unrooted vector was freed");
    assert_eq!((frame.depth, pair.1), (1, 7));
}

// ---- the collection protocol: allocation never collects ---------------------

/// Crossing R_NSize makes a collection due; the next safepoint serves it
/// (memory.c NO_FREE_NODES, at a safepoint instead of in the allocation).
#[test]
fn the_node_count_reaching_r_nsize_makes_a_collection_due() {
    let mut c = Core::new();
    c.set_torture(false);
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert_eq!(n_size, 350_000);
    let n0 = c.stats.collections;
    let nil = c.nil();
    for _ in in_use..n_size {
        node(&mut c, nil, nil);
    }
    assert!(!c.gc_due(), "not due before R_NSize nodes");
    node(&mut c, nil, nil);
    assert!(c.gc_due(), "due at R_NSize nodes");
    assert_eq!(c.poll_word().load(Ordering::Relaxed) & GC_DUE, GC_DUE);
    // more allocations: still no collection
    for _ in 0..1000 {
        node(&mut c, nil, nil);
    }
    assert_eq!(c.stats.collections, n0, "allocation never collects");
    assert!(safepoint(&mut c, &mut ()));
    assert_eq!(c.stats.collections, n0 + 1);
    assert!(!c.gc_due());
    assert_eq!(c.poll_word().load(Ordering::Relaxed), 0);
    assert!(!safepoint(&mut c, &mut ()), "nothing due: a safepoint is a test");
    // the garbage is gone from the count, and the heap kept its size
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert_eq!(n_size, 350_000);
    assert!(in_use < 1_000, "{in_use} nodes in use after the collection");
    assert_eq!(c.stats.collections_by_kind[Safepoint::Return as usize], 1);
}

/// The vector heap triggers on its own: R_VSize is 64 MB of 8-byte VECRECs,
/// and a vector that does not fit in what is free makes a collection due
/// (allocVector3's `VHEAP_FREE() < alloc_size`).
#[test]
fn a_full_vector_heap_makes_a_collection_due() {
    let mut c = Core::new();
    c.set_torture(false);
    let v_size = c.heap_sizes().v_size;
    assert_eq!(v_size, (67_108_864 + 1) / 8);
    // seven 8 MB vectors fit in 64 MB (with what startup took)
    for _ in 0..7 {
        c.alloc_vector(REALSXP, 1 << 20).unwrap();
    }
    assert!(!c.gc_due());
    let valloc = c.heap_sizes().vecrecs_in_use;
    assert!(valloc >= 7 << 20);
    c.alloc_vector(REALSXP, 1 << 20).unwrap();
    assert!(c.gc_due(), "the eighth 8 MB vector makes a collection due");
    assert_eq!(c.stats.collections, 0);
    assert_eq!(c.stats.large_while_due, 0, "the request that crossed the trigger was not made while due");
    safepoint(&mut c, &mut ());
    assert_eq!(c.stats.collections, 1);
}

/// A live heap that outgrows R_NSize grows it (AdjustHeapSize) instead of
/// collecting at every safepoint.
#[test]
fn a_growing_live_heap_grows_r_nsize() {
    let mut c = Core::new();
    c.set_torture(false);
    let nil = c.nil();
    let mut roots = vec![nil];
    for i in 0..1_000_000 {
        roots[0] = node(&mut c, nil, roots[0]);
        safepoint(&mut c, &mut roots);
        if i % 100_000 == 0 {
            assert!(c.stats.collections <= 10, "{} collections", c.stats.collections);
        }
    }
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    assert!(in_use >= 1_000_000);
    assert!(n_size > in_use, "R_NSize {n_size} grew past the live nodes {in_use}");
    assert!(c.stats.collections <= 10, "{} collections for 1M live nodes", c.stats.collections);
}

/// While collections are inhibited a due collection waits: safepoints
/// decline it, and the first one after the inhibition serves it
/// (R_gc_internal's `!R_GCEnabled` branch and gc_pending). The heap sizes
/// do not move meanwhile.
#[test]
fn an_inhibited_collection_waits_for_the_first_safepoint_after() {
    let mut c = Core::new();
    c.set_torture(false);
    let HeapReport { n_size, nodes_in_use: in_use, .. } = c.heap_sizes();
    let n0 = c.stats.collections;
    c.gc_inhibit += 1;
    let nil = c.nil();
    for _ in in_use..n_size + 10 {
        node(&mut c, nil, nil);
    }
    assert!(c.gc_due());
    assert!(!safepoint(&mut c, &mut ()));
    assert_eq!(c.stats.collections, n0);
    assert_eq!(c.heap_sizes().n_size, n_size);
    c.gc_inhibit -= 1;
    assert!(safepoint(&mut c, &mut ()));
    assert_eq!(c.stats.collections, n0 + 1);
}

/// docs/g3.md §3 item 1 under gctorture: every allocation makes a
/// collection due, and none collects (the poll path's debug assertion
/// checks the same in debug builds); the safepoint does.
#[test]
fn allocation_never_collects_under_gctorture() {
    let mut c = Core::new();
    c.set_torture(true);
    let nil = c.nil();
    let n0 = c.stats.collections;
    let mut buf = [1.5f64; 4];
    for i in 0..1_000 {
        c.alloc_vector(INTSXP, 1).unwrap();
        c.alloc_vector(STRSXP, 300).unwrap();
        c.alloc_vector(REALSXP, 20_000).unwrap();
        node(&mut c, nil, nil);
        c.mk_char(format!("torture{i}").as_bytes(), 0).unwrap();
        c.install(format!("torture_sym{}", i % 10).as_bytes()).unwrap();
        unsafe { c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 4, 0, None) }.unwrap();
        assert!(c.gc_due());
    }
    assert_eq!(c.stats.collections, n0, "an allocation collected");
    assert!(safepoint(&mut c, &mut ()));
    assert_eq!(c.stats.collections, n0 + 1);
    // the countdown restarts: the next allocation makes one due again
    assert!(!c.gc_due());
    c.alloc_vector(INTSXP, 1).unwrap();
    assert!(c.gc_due());
    c.set_torture(false);
}

/// gctorture2(step, wait): the first forced collection after `wait`
/// allocations, then one every `step` (memory.c FORCE_GC), each served by
/// the safepoint after the allocation that made it due.
#[test]
fn gctorture_counts_allocations() {
    let mut c = Core::new();
    let collections = |c: &Core| c.stats.collections;
    // one safepoint per allocation, as after each builtin
    let allocs = |c: &mut Core, n: usize| {
        for _ in 0..n {
            c.alloc_vector(INTSXP, 1).unwrap();
            safepoint(c, &mut ());
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
        c.alloc_vector(REALSXP, 10_000).unwrap();
        safepoint(&mut c, &mut ());
    }
    assert_eq!(collections(&c), n2 + 1);
    assert_eq!(c.torture_counters(), (3, 3));
    // while a collection is due the countdown waits (FORCE_GC's
    // `gc_pending ||`): three allocations without a safepoint, one due
    c.set_torture(true);
    let n3 = collections(&c);
    for _ in 0..3 {
        c.alloc_vector(INTSXP, 1).unwrap();
    }
    safepoint(&mut c, &mut ());
    assert_eq!(collections(&c), n3 + 1);
    c.set_torture(false);
}

/// The due bit lives in a poll word, the heap's own or one the embedder
/// polls for other reasons too: the heap sets and clears only GC_DUE.
#[test]
fn the_poll_word_carries_gc_due() {
    const INTERRUPT: u32 = 1 << 8;
    let mut c = Core::new();
    c.set_torture(true);
    c.alloc_vector(INTSXP, 1).unwrap();
    assert_eq!(c.poll_word().load(Ordering::Relaxed), GC_DUE);
    let word = Box::new(AtomicU32::new(INTERRUPT));
    unsafe { c.set_poll_word(std::ptr::NonNull::from(&*word)) };
    assert_eq!(word.load(Ordering::Relaxed), INTERRUPT | GC_DUE, "the due state moved");
    safepoint(&mut c, &mut ());
    assert_eq!(word.load(Ordering::Relaxed), INTERRUPT, "the embedder's bit stays");
    c.alloc_vector(INTSXP, 1).unwrap();
    assert_eq!(word.load(Ordering::Relaxed), INTERRUPT | GC_DUE);
    c.set_torture(false);
    gc(&mut c, &mut ());
    assert_eq!(word.load(Ordering::Relaxed), INTERRUPT);
    drop(c);
}

/// E4's metric: the bytes allocated from the allocation that made a
/// collection due to the collection that served it.
#[test]
fn the_overshoot_runs_from_due_to_the_serving_collection() {
    let mut c = Core::new();
    c.set_torture(false);
    assert_eq!(c.stats.max_overshoot, 0);
    // an explicit collection with nothing due measures nothing
    gc(&mut c, &mut ());
    assert_eq!(c.stats.max_overshoot, 0);
    c.gc_torture(3, 0);
    for _ in 0..2 {
        c.alloc_vector(INTSXP, 1).unwrap();
    }
    assert!(!c.gc_due());
    // the third makes one due: 48 bytes from here (header, length words,
    // one int, rounded to 16), then 100 nodes of 48 bytes
    c.alloc_vector(INTSXP, 1).unwrap();
    assert!(c.gc_due());
    let nil = c.nil();
    for _ in 0..100 {
        node(&mut c, nil, nil);
    }
    c.set_torture(false);
    safepoint(&mut c, &mut ());
    assert_eq!(c.stats.max_overshoot, 48 + 100 * 48);
    assert_eq!(c.stats.total_overshoot, 48 + 100 * 48);
    assert_eq!(c.stats.overshoot_over_bound, 0);
    assert_eq!(robigo_heap::overshoot_bound(0), 32 << 20);
    assert_eq!(robigo_heap::overshoot_bound(1 << 30), (1 << 30) / 10);
}

/// docs/g3.md §3 item 3: a `&Cx` request of LARGE bytes or more while a
/// collection is due is counted; `ROBIGO_GC_STATS=check` makes it panic.
#[test]
fn a_large_request_while_due_is_counted() {
    let mut c = Core::new();
    // counted, not refused: whatever ROBIGO_GC_STATS says (CI's rt job sets
    // `check`, which would make the request below panic)
    c.set_stats_mode(StatsMode::Report);
    c.set_torture(true);
    c.alloc_vector(INTSXP, 1).unwrap();
    assert!(c.gc_due());
    c.set_torture(false);
    // just below LARGE: not counted
    c.alloc_vector(RAWSXP, LARGE - 64).unwrap();
    assert_eq!(c.stats.large_while_due, 0);
    c.alloc_vector(REALSXP, LARGE / 8).unwrap();
    assert_eq!(c.stats.large_while_due, 1);
    assert_eq!(c.stats.largest_while_due, (LARGE + 32) as u64);
    // alloc_rooted is the way: it collects first, and is not counted
    let x = unsafe { c.alloc_rooted(REALSXP, LARGE / 8, &mut ()) }.unwrap();
    assert!(!c.gc_due());
    assert_eq!(c.stats.large_while_due, 1);
    assert_eq!(c.stats.collections_by_kind[Safepoint::Rooted as usize], 1);
    unsafe { assert_eq!(x.std_len() as usize, LARGE / 8) };
}

#[test]
#[should_panic(expected = "while a collection is due")]
fn a_large_request_while_due_panics_under_the_check() {
    let mut c = Core::new();
    c.set_stats_mode(StatsMode::Check);
    c.set_torture(true);
    c.alloc_vector(INTSXP, 1).unwrap();
    c.set_torture(false);
    let _ = c.alloc_vector(REALSXP, LARGE / 8);
}

/// `ROBIGO_GC_STATS`: allocations by SEXPTYPE, collections by safepoint
/// kind.
#[test]
fn statistics_count_allocations_by_type_and_collections_by_kind() {
    let mut c = Core::new();
    c.set_torture(false);
    c.set_stats_mode(StatsMode::Report);
    let nil = c.nil();
    for _ in 0..10 {
        c.alloc_vector(INTSXP, 3).unwrap();
    }
    for _ in 0..5 {
        node(&mut c, nil, nil);
    }
    c.mk_char(b"counted", 0).unwrap();
    unsafe { c.alloc_rooted(REALSXP, 100_000, &mut ()) }.unwrap();
    assert_eq!(c.stats.by_type[INTSXP as usize], (10, 10 * 48));
    assert_eq!(c.stats.by_type[LISTSXP as usize], (5, 5 * 48));
    assert_eq!(c.stats.by_type[CHARSXP as usize], (1, 48));
    assert_eq!(c.stats.by_type[REALSXP as usize], (1, 800_032));
    assert_eq!(c.stats.allocs, 17);
    gc(&mut c, &mut ());
    unsafe { c.safepoint(Safepoint::Loop, &mut ()) };
    c.set_torture(true);
    c.alloc_vector(INTSXP, 1).unwrap();
    c.set_torture(false);
    unsafe { c.safepoint(Safepoint::Loop, &mut ()) };
    assert_eq!(c.stats.collections_by_kind[Safepoint::Explicit as usize], 1);
    assert_eq!(c.stats.collections_by_kind[Safepoint::Loop as usize], 1);
    let r = c.stats_report();
    assert!(
        r.contains(
            "collections 2 (entry 0, loop 1, return 0, poll_alloc 0, alloc_rooted 0, retry 0, export 0, explicit 1)"
        ),
        "{r}"
    );
    assert!(r.contains("INTSXP 11 objects, 528 bytes"), "{r}");
    c.set_stats_mode(StatsMode::Off);
}

// ---- limits, alloc_rooted, NeedGc ------------------------------------------

/// GNU R's allocation errors, verbatim (memory.c mem_err_heap,
/// mem_err_cons, allocVector3).
#[test]
fn allocation_errors_say_what_gnu_r_says() {
    let m = |e: AllocError| e.message();
    assert_eq!(m(AllocError::VectorMemory { limit: None }), "vector memory exhausted");
    assert_eq!(
        m(AllocError::VectorMemory { limit: Some(64 << 20) }),
        "vector memory limit of 64.0 Mb reached, see mem.maxVSize()"
    );
    assert_eq!(
        m(AllocError::VectorMemory { limit: Some(16 << 30) }),
        "vector memory limit of 16.0 Gb reached, see mem.maxVSize()"
    );
    assert_eq!(
        m(AllocError::VectorMemory { limit: Some(512 << 10) }),
        "vector memory limit of 512.0 Kb reached, see mem.maxVSize()"
    );
    assert_eq!(m(AllocError::ConsMemory { limit: None }), "cons memory exhausted");
    assert_eq!(
        m(AllocError::ConsMemory { limit: Some(500_000) }),
        "cons memory limit of 500000 nodes reached, see mem.maxNSize()"
    );
    assert_eq!(m(AllocError::CannotAllocate { vecrecs: 1 << 37 }), "cannot allocate vector of size 1024.0 Gb");
    assert_eq!(m(AllocError::CannotAllocate { vecrecs: 3 << 18 }), "cannot allocate vector of size 6.0 Mb");
    assert_eq!(m(AllocError::CannotAllocate { vecrecs: 1000 }), "cannot allocate vector of size 8 Kb");
}

/// `NeedGc` is one word, so a fallible allocation's result comes back in
/// two registers.
#[test]
fn need_gc_is_one_word() {
    assert_eq!(std::mem::size_of::<Result<Sexp, NeedGc>>(), 16);
    let r = NeedGc::new(STRSXP, 1 << 52);
    assert_eq!((r.sexptype(), r.length()), (STRSXP, 1 << 52));
    assert_eq!(NeedGc::new(OBJSXP, 0).sexptype(), OBJSXP);
    assert_eq!(NeedGc::new(REALSXP, usize::MAX).length(), (1 << 56) - 1);
    assert_eq!(format!("{:?}", NeedGc::new(INTSXP, 3)), "NeedGc { sexptype: 13, length: 3 }");
}

/// A limit is checked against everything allocated under `&Cx` (only a
/// collection tells garbage apart): the poll-only path refuses with the
/// request, `make_room` collects, and the retry passes. A request past the
/// limit on live data alone fails with GNU R's error.
#[test]
fn a_vector_limit_refuses_under_cx_and_collects_under_mut_cx() {
    let mut c = Core::new();
    c.set_torture(false);
    let v_bytes = c.heap_sizes().v_size * 8;
    assert!(!c.set_max_vsize(Some(v_bytes / 2)), "a limit below the current heap is refused");
    assert!(c.set_max_vsize(Some(v_bytes)));
    assert_eq!(c.max_vsize(), Some(v_bytes));
    // 56 MB of garbage, then a 16 MB request: past 64 MB with the garbage
    for _ in 0..7 {
        c.alloc_vector(REALSXP, 1 << 20).unwrap();
    }
    let keep = c.alloc_vector(INTSXP, 1).unwrap();
    let req = c.alloc_vector(REALSXP, 2 << 20).unwrap_err();
    assert_eq!(req, NeedGc::new(REALSXP, 2 << 20));
    let n0 = c.stats.collections;
    let mut roots = vec![keep];
    unsafe { c.make_room(req, &mut roots) }.unwrap();
    assert_eq!(c.stats.collections, n0 + 1);
    assert_eq!(c.stats.collections_by_kind[Safepoint::Retry as usize], 1);
    c.alloc_vector(REALSXP, 2 << 20).unwrap();
    // live data past the limit: GNU R's error after the collection
    let big = unsafe { c.alloc_rooted(REALSXP, 10 << 20, &mut roots) }.unwrap_err();
    assert_eq!(big, AllocError::VectorMemory { limit: Some(v_bytes) });
    assert_eq!(big.message(), "vector memory limit of 64.0 Mb reached, see mem.maxVSize()");
    unsafe { assert_eq!(ints(keep).len(), 1) };
    assert!(c.set_max_vsize(None));
    assert_eq!(c.max_vsize(), None);
    unsafe { c.alloc_rooted(REALSXP, 10 << 20, &mut roots) }.unwrap();
}

#[test]
fn a_node_limit_refuses_under_cx_and_fails_on_live_nodes() {
    let mut c = Core::new();
    c.set_torture(false);
    let n_size = c.heap_sizes().n_size;
    assert!(!c.set_max_nsize(Some(n_size - 1)));
    assert!(c.set_max_nsize(Some(n_size)));
    let nil = c.nil();
    let in_use = c.heap_sizes().nodes_in_use;
    let mut list = nil;
    for _ in in_use..n_size {
        list = node(&mut c, nil, list);
    }
    let req = unsafe { c.alloc_node(LISTSXP, nil, list, nil) }.unwrap_err();
    assert_eq!(req, NeedGc::new(LISTSXP, 1));
    // the list is live: no room after the collection either
    let mut roots = vec![list];
    let e = unsafe { c.make_room(req, &mut roots) }.unwrap_err();
    assert_eq!(e, AllocError::ConsMemory { limit: Some(n_size) });
    assert_eq!(e.message(), format!("cons memory limit of {n_size} nodes reached, see mem.maxNSize()"));
    // dropped: the retry finds room
    unsafe { c.make_room(req, &mut ()) }.unwrap();
    unsafe { c.alloc_node(LISTSXP, nil, nil, nil) }.unwrap();
    // with collections inhibited GNU R checks no node limit (R_gc_internal's
    // `!R_GCEnabled` branch grows R_NSize): nor does the poll-only path
    c.gc_inhibit += 1;
    let in_use = c.heap_sizes().nodes_in_use;
    for _ in in_use..n_size + 10 {
        unsafe { c.alloc_node(LISTSXP, nil, nil, nil) }.unwrap();
    }
    c.gc_inhibit -= 1;
    assert!(c.gc_due());
}

/// memory.c:2709-2714: `alloc_rooted` collects before a request that
/// would make a collection due, so the garbage is gone before the large
/// result exists (GNU R's peak), where `&Cx` allocation would only set the
/// due bit.
#[test]
fn alloc_rooted_collects_before_a_request_that_crosses_the_trigger() {
    let mut c = Core::new();
    c.set_torture(false);
    for _ in 0..7 {
        c.alloc_vector(REALSXP, 1 << 20).unwrap();
    }
    assert!(!c.gc_due());
    let before = c.heap_reserved();
    let x = unsafe { c.alloc_rooted(REALSXP, 1 << 20, &mut ()) }.unwrap();
    assert_eq!(c.stats.collections_by_kind[Safepoint::Rooted as usize], 1);
    assert!(!c.gc_due());
    // the seven garbage vectors' chunks went back before the eighth came
    assert!(c.heap_reserved() < before, "{} >= {before}", c.heap_reserved());
    unsafe { assert_eq!(x.std_len(), 1 << 20) };
    // below the trigger it does not collect
    unsafe { c.alloc_rooted(INTSXP, 10, &mut ()) }.unwrap();
    assert_eq!(c.stats.collections, 1);
}

#[test]
fn a_size_past_the_address_space_is_refused_with_gnu_rs_message() {
    let mut c = Core::new();
    let req = c.alloc_vector(REALSXP, usize::MAX / 4).unwrap_err();
    let e = unsafe { c.make_room(req, &mut ()) }.unwrap_err();
    assert!(matches!(e, AllocError::CannotAllocate { .. }));
    assert!(e.message().starts_with("cannot allocate vector of size "), "{}", e.message());
    let e = unsafe { c.alloc_rooted(REALSXP, usize::MAX / 4, &mut ()) }.unwrap_err();
    assert!(matches!(e, AllocError::CannotAllocate { .. }));
}

// ---- tracing --------------------------------------------------------------

/// Records the offsets of the edges an object shows.
struct Offsets {
    base: usize,
    seen: Vec<usize>,
}

impl Visitor for Offsets {
    fn edge(&mut self, edge: &mut Sexp) {
        self.seen.push(edge as *mut Sexp as usize - self.base);
    }
}

fn traced_offsets(x: Sexp) -> Vec<usize> {
    let mut v = Offsets { base: x.addr(), seen: Vec::new() };
    unsafe { x.trace_fields(&mut v) };
    v.seen.sort_unstable();
    v.seen
}

/// The pattern for every trace arm (`Sexp::trace_fields`): an object of a
/// kind shows exactly its pointer words, once each. N1 (slot frames, per
/// slot kind, and their overflow area), C1 (PROMSXP with PROMISE_TAG) and
/// V1 (code objects) add their rows.
#[test]
fn each_type_traces_exactly_its_pointer_words() {
    let mut c = Core::new();
    let nil = c.nil();
    let node_words = vec![8, 16, 24, 32, 40];
    for t in [LISTSXP, LANGSXP, DOTSXP, CLOSXP, ENVSXP, PROMSXP, BCODESXP, OBJSXP] {
        let x = unsafe { c.alloc_node(t, nil, nil, nil) }.unwrap();
        assert_eq!(traced_offsets(x), node_words, "SEXPTYPE {t}");
    }
    // a symbol's 4th word is its id and hash, not an object
    assert_eq!(traced_offsets(c.install(b"traced").unwrap()), vec![8, 16, 24, 32], "SYMSXP");
    // an external pointer's car is an address
    let p = unsafe { c.alloc_node(EXTPTRSXP, Sexp(16 as *mut u8), nil, nil) }.unwrap();
    assert_eq!(traced_offsets(p), vec![8, 24, 32, 40], "EXTPTRSXP");
    for (t, n) in [(STRSXP, 3), (VECSXP, 2), (EXPRSXP, 1), (STRSXP, 300)] {
        let x = c.alloc_vector(t, n).unwrap();
        let want: Vec<usize> = std::iter::once(8).chain((0..n).map(|i| 32 + 8 * i)).collect();
        assert_eq!(traced_offsets(x), want, "SEXPTYPE {t}, length {n}");
    }
    for t in [LGLSXP, INTSXP, REALSXP, CPLXSXP, RAWSXP] {
        assert_eq!(traced_offsets(c.alloc_vector(t, 4).unwrap()), vec![8], "SEXPTYPE {t}");
    }
    // a weak reference's fields are the weak pass's
    assert_eq!(traced_offsets(c.alloc_vector(WEAKREFSXP, 4).unwrap()), vec![8], "WEAKREFSXP");
    // a CHARSXP has a hash where attributes go
    assert_eq!(traced_offsets(c.mk_char(b"no edges", 0).unwrap()), Vec::<usize>::new(), "CHARSXP");
    // an ALTREP object: data1, data2, class and the 4th word
    let a = unsafe { c.alloc_node(LISTSXP, nil, nil, nil) }.unwrap();
    unsafe {
        a.set_typ(INTSXP);
        a.set_flag(flags::ALTREP, true);
    }
    assert_eq!(traced_offsets(a), node_words, "ALTREP");
}

// ---- C segments: no Rust frame is ever scanned ------------------------------

/// A dead object whose only references are a Rust local's stale copies is
/// freed: Rust frames are not scanned.
#[test]
fn rust_locals_keep_nothing() {
    let mut c = Core::new();
    let x = int_vec(&mut c, &[42; 64]);
    let interior = std::hint::black_box(unsafe { (x.std_data() as *mut i32).add(10) });
    c.set_torture(true);
    gc(&mut c, &mut ());
    assert!(c.resolve(x.addr()).is_none(), "a Rust local kept an object alive");
    std::hint::black_box((x, interior));
    c.set_torture(false);
}

/// A C local keeps what it points at, an interior pointer included.
#[test]
fn c_words_keep_objects_alive() {
    let mut c = Core::new();
    let x = int_vec(&mut c, &[42; 64]);
    let interior = unsafe { (x.std_data() as *mut i32).add(10) } as usize;
    let x_addr = x.addr();
    for i in 0..200_000 {
        int_vec(&mut c, &[i]);
    }
    collect_holding(&mut c, &[interior]);
    assert_eq!(c.resolve(interior).map(|s| s.addr()), Some(x_addr));
    unsafe { assert_eq!(*(interior as *const i32), 42) };
}

#[test]
#[should_panic(expected = "segment open")]
fn a_collection_with_a_c_segment_open_panics() {
    let mut c = Core::new();
    let m = 0usize;
    c.set_scan_top(std::hint::black_box(&m) as *const usize as usize);
    gc(&mut c, &mut ());
}

/// A routine's stack-passed parameters are C locals: a fresh object kept
/// only in its 7th and 65th parameter survives a collection, while the
/// trampoline opens the C segment at the routine's stack arguments
/// (`shim.c`) and the collecting export closes it. The first narrowing,
/// from a sampled stack pointer, left them out with Clang.
#[test]
fn stack_parameters_of_a_routine_are_roots() {
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    unsafe {
        native_call(core, robigo_heap_test_stack_params as *const c_void, &[]);
        let hidden = std::ptr::read_volatile(&raw const robigo_heap_test_hidden);
        assert!((*core).resolve(!hidden[0]).is_some(), "the 7th parameter's object was freed");
        assert!((*core).resolve(!hidden[1]).is_some(), "the 65th parameter's object was freed");
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}

unsafe extern "C" fn probe_sp() -> Sexp {
    let m = 0usize;
    ROUTINE_SP.set(std::hint::black_box(&m) as *const usize as usize);
    Sexp::NULL
}

unsafe extern "C" fn collect_now() -> Sexp {
    routine_collect();
    Sexp::NULL
}

/// Fill the stack below the caller's frame with `w` in `[lo, hi)` and with
/// zeros elsewhere: stale words where the next call's frames will be.
#[inline(never)]
fn plant(w: usize, lo: usize, hi: usize) {
    let mut a = [0usize; 2048];
    let span = std::mem::size_of_val(&a);
    assert!(hi - lo <= span, "the frames to plant ({} bytes) exceed the plant buffer ({span})", hi - lo);
    let mut planted = 0;
    for x in a.iter_mut() {
        let at = x as *mut usize as usize;
        if at >= lo && at < hi {
            *x = w;
            planted += 1;
        }
    }
    std::hint::black_box(&mut a);
    // every word of [lo, hi) planted: a buffer elsewhere would plant
    // nothing, and the test would pass without testing anything
    assert_eq!(planted, (hi - lo) / 8, "the plant buffer does not cover [{lo:#x}, {hi:#x})");
}

/// A weak reference whose key nothing reaches: the key's address,
/// inverted (no word on the stack points at it), and the reference.
#[inline(never)]
fn dead_key(c: &mut Core) -> (usize, Sexp) {
    let key = c.alloc_vector(INTSXP, 1).unwrap();
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let w = weak(c, key, fin);
    (!key.addr(), w)
}

/// The words a native call leaves unwritten between the caller's frame and
/// the routine's frame read as anything but a pointer to a dead object
/// (weak-keys-collections lost a finalizer to one in the trampoline's
/// `jmp_buf`). Ported from robigo-capi: x86-64 only under today's order
/// (elsewhere the segment opens at the trampoline's frame, whose stale
/// words legitimately keep the object); with `ROBIGO_GC_PIN_ONLY=1` the C
/// words decide no weak reference, and it holds everywhere.
#[test]
fn native_call_frames_keep_nothing() {
    let pin_only = std::env::var_os("ROBIGO_GC_PIN_ONLY").is_some_and(|v| v == "1");
    let plain = std::env::var_os("ROBIGO_CALL65_PLAIN").is_some_and(|v| v == "1");
    if !pin_only && (plain || !cfg!(target_arch = "x86_64")) {
        eprintln!(
            "skipped: without the stub's exact segment top (x86-64), only ROBIGO_GC_PIN_ONLY=1 ignores stale C words"
        );
        return;
    }
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    unsafe {
        // where the frames are: the caller's, down to the routine's
        let top = native_call(core, probe_sp as *const c_void, &[]);
        let routine = ROUTINE_SP.get();
        assert!(routine < top);
        let (hidden, w) = dead_key(&mut *core);
        // every word of those frames is a pointer to the dead key, as
        // earlier deeper frames (the evaluator's) leave them
        plant(!hidden, routine, top);
        assert_eq!(native_call(core, collect_now as *const c_void, &[]), top, "same frames");
        assert_eq!(
            (*core).pop_finalizable(),
            Some(w),
            "a stale word in the frames of a native call kept an object alive"
        );
        std::hint::black_box(hidden);
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}

/// The six-argument native entry (ALTREP methods, `R_ToplevelExec`'s fun,
/// C finalizers, cends) opens a C segment too: an object only a local of
/// the callee's frame holds survives a collection the callee makes through
/// an export.
#[test]
fn a_six_argument_entrys_locals_are_roots() {
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    unsafe {
        native_call6(core, robigo_heap_test_local6 as *const c_void, [0; 6]);
        let hidden = std::ptr::read_volatile(&raw const robigo_heap_test_hidden);
        assert!((*core).resolve(!hidden[0]).is_some(), "the local's object was freed");
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}

/// ... and its segment starts at the call: the words the six-argument
/// trampoline's frame and the caller's leave between them and the
/// callee's frame keep nothing (x86-64, whose stub opens the segment
/// exactly; elsewhere only `ROBIGO_GC_PIN_ONLY=1` ignores stale C words).
#[test]
fn six_argument_entry_frames_keep_nothing() {
    let pin_only = std::env::var_os("ROBIGO_GC_PIN_ONLY").is_some_and(|v| v == "1");
    if !pin_only && !cfg!(target_arch = "x86_64") {
        eprintln!(
            "skipped: without the stub's exact segment top (x86-64), only ROBIGO_GC_PIN_ONLY=1 ignores stale C words"
        );
        return;
    }
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    unsafe {
        let top = native_call6(core, probe_sp as *const c_void, [0; 6]);
        let routine = ROUTINE_SP.get();
        assert!(routine < top);
        let (hidden, w) = dead_key(&mut *core);
        plant(!hidden, routine, top);
        assert_eq!(native_call6(core, collect_now as *const c_void, [0; 6]), top, "same frames");
        assert_eq!(
            (*core).pop_finalizable(),
            Some(w),
            "a stale word in the frames of a six-argument native call kept an object alive"
        );
        std::hint::black_box(hidden);
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}

/// The export's own Rust frame is not in the segment: a dead key's address
/// in every word of it keeps nothing (x86-64, whose export stand-in has the
/// exact machine entry).
#[test]
fn export_frames_keep_nothing() {
    if !cfg!(target_arch = "x86_64") {
        eprintln!("skipped: the export stand-in has an exact machine entry on x86-64 only");
        return;
    }
    let mut c = Core::new();
    c.set_torture(false);
    let core: *mut Core = &mut *c;
    unsafe {
        let (hidden, w) = dead_key(&mut *core);
        PLANT.set(!hidden);
        native_call(core, robigo_heap_test_collect65 as *const c_void, &[]);
        PLANT.set(0);
        assert_eq!((*core).pop_finalizable(), Some(w), "a word of the export's frame kept an object alive");
    }
    ROUTINE_CORE.set(std::ptr::null_mut());
}

/// Weak references and finalizers are decided by the order in
/// `finish_collection`: with `ROBIGO_GC_PIN_ONLY=1` (docs/g3.md §3, the
/// target), a key that only a C word reaches is ready to finalize, and
/// its memory stays valid for the C code that still names it; without
/// it (today, until M0's verdict), the C word keeps the key alive.
#[test]
fn a_key_only_a_c_word_reaches() {
    let pin_only = std::env::var_os("ROBIGO_GC_PIN_ONLY").is_some_and(|v| v == "1");
    let mut c = Core::new();
    c.set_torture(false);
    let key = int_vec(&mut c, &[7]);
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let w = weak(&mut c, key, fin);
    collect_holding(&mut c, &[key.addr()]);
    assert!(c.resolve(key.addr()).is_some_and(|k| k == key), "a C word's object was freed");
    unsafe { assert_eq!(ints(key), &[7]) };
    if pin_only {
        assert_eq!(c.pop_finalizable(), Some(w), "pins decide no finalizer");
    } else {
        assert_eq!(c.pop_finalizable(), None, "a strong C word keeps the key");
        assert_eq!(c.weak_refs(), &[w]);
    }
}

// ---- the conservative scan's word rules (through C segments) ---------------

/// Allocate a 4-double vector and its neighbour; return only the
/// one-past-the-end pointer of the first.
#[inline(never)]
fn alloc_with_end_pointer(c: &mut Core) -> usize {
    let x = c.alloc_vector(REALSXP, 4).unwrap();
    unsafe {
        for i in 0..4 {
            *(x.std_data() as *mut f64).add(i) = 1.5;
        }
    }
    let n = c.alloc_vector(REALSXP, 4).unwrap();
    assert_eq!(n.addr(), x.addr() + 64, "adjacent slots");
    std::hint::black_box(n);
    unsafe { x.std_data() as usize + 32 }
}

#[test]
fn one_past_the_end_keeps_the_object() {
    let mut c = Core::new();
    c.set_torture(false);
    // 4 doubles: 32 + 32 = 64 bytes, a whole number of granules
    let end = alloc_with_end_pointer(&mut c);
    // poison freed slots so a wrongly freed object is visible
    c.set_torture(true);
    collect_holding(&mut c, &[end]);
    collect_holding(&mut c, &[end]);
    assert_eq!(unsafe { *((end - 8) as *const f64) }, 1.5, "only a one-past-the-end pointer referenced it");
    c.set_torture(false);
}

/// As [`alloc_with_end_pointer`], then shorten the vector in place
/// (`SETLENGTH`, optionally growable as `SET_GROWABLE_BIT` makes it): C
/// keeps the end of the buffer it allocated, not of the length.
#[inline(never)]
fn alloc_shortened_with_end_pointer(c: &mut Core, growable: bool) -> usize {
    let x = c.alloc_vector(REALSXP, 4).unwrap();
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
    let n = c.alloc_vector(REALSXP, 4).unwrap();
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
        c.set_torture(true);
        collect_holding(&mut c, &[end]);
        collect_holding(&mut c, &[end]);
        assert!(c.resolve(end - 64).is_some(), "growable {growable}: only the buffer's end pointer referenced it");
        assert_eq!(unsafe { *((end - 8) as *const f64) }, 1.5, "growable {growable}");
        c.set_torture(false);
    }
}

/// A shortened large vector whose data ends exactly at its chunk's end
/// (`DATA_OFF + OFF_DATA + 16,246 * 8 = 131,072`); its end pointer.
#[inline(never)]
fn alloc_large_ending_at_its_chunk(c: &mut Core) -> usize {
    let x = c.alloc_vector(REALSXP, 16_246).unwrap();
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
    collect_holding(&mut c, &[end]);
    assert!(c.resolve(end - 1).is_some(), "only the end pointer at the heap's top referenced it");
    assert_eq!(unsafe { *((end - 8) as *const f64) }, 1.5);
}

/// Allocate an external pointer and its neighbour; return the neighbour
/// (whose address is one past the pointer's slot) and the pointer's
/// address in a form the scan cannot read.
#[inline(never)]
fn alloc_with_neighbour(c: &mut Core) -> (Sexp, usize) {
    let nil = c.nil();
    let x = unsafe { c.alloc_node(EXTPTRSXP, Sexp::NULL, nil, nil) }.unwrap();
    let n = node(c, nil, nil);
    assert_eq!(n.addr(), x.addr() + robigo_heap::NODE_SIZE, "adjacent slots");
    (n, !x.addr())
}

#[test]
fn a_pointer_to_the_next_object_does_not_keep_a_node() {
    let mut c = Core::new();
    // the two allocations must be adjacent: no collection between them
    c.set_torture(false);
    let (n, hidden) = alloc_with_neighbour(&mut c);
    // the C word is `n`, the word one past the pointer's slot: a node has
    // no data a C loop runs to the end of, so the word is `n`'s alone
    collect_holding(&mut c, &[n.addr()]);
    assert!(c.resolve(n.addr()).is_some(), "the neighbour is referenced");
    assert!(c.resolve(!hidden).is_none(), "the external pointer is garbage");
}

// ---- heap layout and sweeping ---------------------------------------------

#[test]
fn dead_chars_leave_the_cache() {
    let base = baseline_live();
    let mut c = Core::new();
    let cached = c.char_cache_len();
    for i in 0..50_000 {
        c.mk_char(format!("s{i}").as_bytes(), 0).unwrap();
    }
    gc(&mut c, &mut ());
    assert_eq!(c.stats.live_objects, base, "every new string died");
    assert_eq!(c.char_cache_len(), cached);
    // a fresh string gets a fresh object
    let a = c.mk_char(b"s123", 0).unwrap();
    unsafe { assert_eq!(a.char_bytes(), b"s123") };
}

#[test]
fn preserve_and_release() {
    let mut c = Core::new();
    let x = c.alloc_vector(REALSXP, 4).unwrap();
    let addr = x.addr();
    unsafe {
        c.preserve(x);
    }
    for _ in 0..3 {
        gc(&mut c, &mut ());
    }
    assert_eq!(c.resolve(addr), Some(x));
    c.release(x);
    gc(&mut c, &mut ());
    assert!(c.resolve(addr).is_none());
}

static RELEASED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
unsafe extern "C" fn count_release(owner: usize) {
    RELEASED.fetch_add(owner, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn extdata_release_runs_once_dead() {
    let mut c = Core::new();
    let mut buf = vec![1.5f64, 2.5];
    // SAFETY: `buf` outlives these objects and count_release never reads it.
    let live = unsafe { c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1000, Some(count_release)) }.unwrap();
    for _ in 0..10 {
        unsafe {
            c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 1, Some(count_release)).unwrap();
        }
    }
    let mut roots = vec![live];
    for _ in 0..3 {
        gc(&mut c, &mut roots);
    }
    // release callbacks wait for a safe point
    assert_eq!(RELEASED.load(std::sync::atomic::Ordering::SeqCst), 0);
    for (f, h) in c.take_releases() {
        unsafe { f(h) };
    }
    assert_eq!(RELEASED.load(std::sync::atomic::Ordering::SeqCst), 10, "every dead extdata released, the live one not");
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
    let v = c.alloc_vector(INTSXP, 1).unwrap();
    let nil = c.nil();
    let n = node(&mut c, v, nil);
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
        let ch = c.mk_char(b"x", 0).unwrap();
        assert_eq!(std::ffi::CStr::from_ptr(ch.0.add(32) as *const std::ffi::c_char).to_bytes(), b"x");
    }
}

#[test]
fn symbol_values_are_traced() {
    let mut c = Core::new();
    let s = c.install(b"some_symbol").unwrap();
    let v = c.alloc_vector(REALSXP, 3).unwrap();
    let addr = v.addr();
    unsafe { s.set_cdr(v) };
    for _ in 0..3 {
        gc(&mut c, &mut ());
    }
    for i in 0..100_000 {
        let x = c.alloc_vector(REALSXP, 3).unwrap();
        unsafe { *(x.std_data() as *mut f64) = i as f64 };
    }
    unsafe { assert_eq!(s.cdr().addr(), addr) };
    assert!(c.resolve(addr).is_some());
}

/// A registered weak reference (key, value nil, finalizer).
fn weak(c: &mut Core, key: Sexp, fin: Sexp) -> Sexp {
    let w = c.alloc_vector(WEAKREFSXP, 4).unwrap();
    unsafe {
        let data = w.std_data() as *mut Sexp;
        *data = key;
        *data.add(2) = fin;
        c.register_weak_ref(w);
    }
    w
}

#[test]
fn weak_refs_queue_finalizers_of_dead_keys() {
    let mut c = Core::new();
    let key_live = c.alloc_vector(INTSXP, 1).unwrap();
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let w1 = weak(&mut c, key_live, fin);
    let dead_key = c.alloc_vector(INTSXP, 1).unwrap();
    let w2 = weak(&mut c, dead_key, fin);
    let mut roots = vec![key_live, w1];
    gc(&mut c, &mut roots);
    assert_eq!(c.pop_finalizable(), Some(w2), "only the dead key's reference is ready");
    assert!(c.pop_finalizable().is_none());
    unsafe {
        assert_eq!(*(w2.std_data() as *const Sexp), dead_key, "key kept for the finalizer");
        assert!(w2.gp() & gp::READY_TO_FINALIZE != 0);
    }
    assert_eq!(c.weak_refs(), &[w1]);
}

#[test]
fn finalizers_participate_in_the_weak_reference_fixed_point() {
    let mut c = Core::new();
    let key = c.alloc_vector(INTSXP, 1).unwrap();
    let finalizer = c.alloc_vector(VECSXP, 1).unwrap();
    let other_key = c.alloc_vector(INTSXP, 1).unwrap();
    let other_fin = c.alloc_vector(RAWSXP, 8).unwrap();
    // only the first reference's finalizer reaches the second one's key
    unsafe { *(finalizer.std_data() as *mut Sexp) = other_key };
    let first = weak(&mut c, key, finalizer);
    let second = weak(&mut c, other_key, other_fin);
    gc(&mut c, &mut vec![key]);
    assert!(c.pop_finalizable().is_none());
    assert_eq!(c.weak_refs().len(), 2);
    gc(&mut c, &mut ());
    assert_eq!(c.pop_finalizable(), Some(second));
    assert_eq!(c.pop_finalizable(), Some(first));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn manually_finalized_weak_reference_is_removed_even_with_nil_key() {
    let mut c = Core::new();
    let nil = c.nil();
    let w = weak(&mut c, nil, nil);
    unsafe {
        w.set_gp(w.gp() | gp::READY_TO_FINALIZE);
    }
    gc(&mut c, &mut ());
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
    let mut c = Core::new();
    let key = c.alloc_vector(INTSXP, 1).unwrap();
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let first = weak(&mut c, key, fin);
    gc(&mut c, &mut ());
    assert_eq!(c.pop_finalizable(), Some(first));
    // A finalizer registers another key while the batch is running.
    let later_key = c.alloc_vector(INTSXP, 1).unwrap();
    let later = weak(&mut c, later_key, fin);
    assert_eq!(c.weak_refs(), &[later], "the new reference is not lost");
    // Its key dies: the next collection queues it.
    gc(&mut c, &mut ());
    assert_eq!(c.pop_finalizable(), Some(later));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn all_finalizers_of_a_dead_key_are_queued_in_the_same_collection() {
    let mut c = Core::new();
    let key = c.alloc_vector(INTSXP, 1).unwrap();
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let first = weak(&mut c, key, fin);
    let second = weak(&mut c, key, fin);
    gc(&mut c, &mut ());
    assert_eq!(c.pop_finalizable(), Some(second));
    assert_eq!(c.pop_finalizable(), Some(first));
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn exit_finalizers_queue_in_weak_reference_list_order() {
    let mut c = Core::new();
    let first_key = c.alloc_vector(INTSXP, 1).unwrap();
    let second_key = c.alloc_vector(INTSXP, 1).unwrap();
    let fin = c.alloc_vector(RAWSXP, 8).unwrap();
    let first = weak(&mut c, first_key, fin);
    let second = weak(&mut c, second_key, fin);
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
    gc(&mut c, &mut vec![first_key]);
    assert!(c.pop_finalizable().is_none());
}

#[test]
fn medium_vectors_share_blocks() {
    // 10,000 live 1000-double vectors (8 KiB of payload each): before the
    // medium classes each took a 64 KiB chunk (655 MB reserved)
    let mut c = Core::new();
    let keep = c.alloc_vector(VECSXP, 10_000).unwrap();
    let mut roots = vec![keep];
    // a safepoint after each: under ROBIGO_GCTORTURE that collects every
    // time, which checks that collections do not abandon a block's space
    for i in 0..10_000 {
        let v = c.alloc_vector(REALSXP, 1000).unwrap();
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
        safepoint(&mut c, &mut roots);
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
    let mut c = Core::new();
    // a measurement over a large live heap: no collection per allocation
    c.set_torture(false);
    let n = 400_000;
    let keep = c.alloc_vector(VECSXP, n).unwrap();
    let mut roots = vec![keep];
    for i in 0..n {
        // 512-byte payloads: ~200 MB live
        let v = c.alloc_vector(REALSXP, 64).unwrap();
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
    }
    gc(&mut c, &mut roots);
    let live_mb = c.heap_reserved() >> 20;
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe failed");
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        let before = private_dirty_kb();
        gc(&mut c, &mut roots);
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
    let mut c = Core::new();
    let keep = c.alloc_vector(VECSXP, 2_000).unwrap();
    let mut roots = vec![keep];
    for i in 0..2_000 {
        let v = c.alloc_vector(REALSXP, 1000).unwrap();
        unsafe { *(keep.std_data() as *mut Sexp).add(i) = v };
        // a collection after every allocation: each stranded the rest of
        // the current block before the fix (2,000 blocks)
        gc(&mut c, &mut roots);
    }
    let blocks = c.heap_reserved() / (64 << 10);
    assert!(blocks < 400, "{blocks} blocks for 2,000 x 8 KiB live vectors (8 per block)");
}

/// A large object's mark word lives in its chunk's header. The chunk comes
/// from `malloc`, so if the word started out containing the next collection's
/// epoch the object would look marked already and its children would not
/// be traced.
#[test]
fn a_large_object_starts_unmarked_whatever_its_chunk_held() {
    let mut c = Core::new();
    c.set_torture(false);
    let next_epoch = c.stats.collections + 1;
    CHUNK_WORD.with(|w| w.set(next_epoch));
    let big = c.alloc_vector(VECSXP, 100_000).unwrap();
    CHUNK_WORD.with(|w| w.set(0));
    let child = c.alloc_vector(INTSXP, 1).unwrap();
    unsafe { *(big.std_data() as *mut Sexp) = child };
    gc(&mut c, &mut vec![big]);
    assert_eq!(c.stats.collections, next_epoch);
    assert!(c.resolve(child.addr()).is_some(), "the large vector's child was swept");
}

#[test]
#[should_panic(expected = "not a vector type")]
fn alloc_vector_rejects_a_non_vector_type() {
    let _ = Core::new().alloc_vector(LISTSXP, 5);
}

#[test]
#[should_panic(expected = "not a vector type")]
fn a_long_vector_of_a_non_vector_type_is_rejected_too() {
    let _ = Core::new().alloc_vector(ENVSXP, 1000);
}

#[test]
#[should_panic(expected = "not a vector type")]
fn alloc_rooted_rejects_a_non_vector_type() {
    let _ = unsafe { Core::new().alloc_rooted(ENVSXP, 1000, &mut ()) };
}

/// A C++ `throw` is `_Unwind_RaiseException`, and `__cxa_throw` calls
/// `std::terminate` when that returns. It returns end-of-stack only if phase
/// 1 stops at the shim; otherwise it finds a handler above the shim
/// (libtest's `catch_unwind` here, a package's `catch` in real life) and
/// phase 2 takes the exception through Rust frames.
#[cfg(target_os = "linux")]
#[test]
fn a_foreign_exception_stops_at_the_c_shim() {
    use std::ffi::c_int;
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
    let mut jb = robigo_heap::unwind::JmpBuf::UNSET;
    let unwound = unsafe {
        robigo_heap::unwind::robigo_heap_try_call65(
            &mut jb,
            thrower as *const c_void,
            args.as_mut_ptr(),
            &mut out,
            std::ptr::null_mut(),
            0,
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
    let mut c = Core::new();
    c.set_torture(false);
    let keep = c.alloc_vector(VECSXP, SLOTS).unwrap();
    let mut roots = vec![keep];
    let slots = unsafe { keep.std_data() as *mut Sexp };
    for i in 0..SLOTS {
        let v = c.alloc_vector(REALSXP, LEN).unwrap();
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
    gc(&mut c, &mut roots);
    // one more death: this sweep walks the block
    drop_slot(&mut c, 1);
    gc(&mut c, &mut roots);
    for _ in 0..freed.len() {
        let x = c.alloc_vector(REALSXP, LEN).unwrap();
        assert!(freed.remove(&x.addr()), "{} free slots were lost", freed.len());
    }
}

/// An immortal CHARSXP is counted once by `immortal_chars`; a store into it
/// must not count it again as marked, or the sum can equal the table's size
/// with a dead entry in it, and the prune is skipped.
#[test]
fn a_store_into_an_immortal_char_does_not_hide_dead_cache_entries() {
    let mut c = Core::new();
    c.set_torture(false);
    let blank = c.blank_string();
    // the barrier fires on the header alone; the slot is a local, as no
    // CHARSXP field is a traced one
    let mut slot = c.nil();
    unsafe { blank.store(&mut slot, c.nil()) };
    let before = c.char_cache_len();
    c.mk_char(b"soon dead", 0).unwrap();
    assert_eq!(c.char_cache_len(), before + 1);
    gc(&mut c, &mut ());
    assert_eq!(c.char_cache_len(), before, "a dead CHARSXP stayed in the cache");
}

/// A symbol's 4th word holds its id, dense in the order the heap made its
/// symbols (R_UnboundValue 0, R_MissingArg 1, then installs), and GNU R's
/// `R_Newhashpjw` of its name, which install also leaves on the name
/// (HASHASH, HASHVALUE in TRUELENGTH).
#[test]
fn symbols_carry_an_id_and_gnu_rs_hash() {
    let mut c = Core::new();
    unsafe {
        assert_eq!(c.unbound().sym_id(), 0);
        assert_eq!(c.missing_arg().sym_id(), 1);
        let n0 = c.symbol_count() as u32;
        let x = c.install(b"x").unwrap();
        let y = c.install(b"y").unwrap();
        assert_eq!(x.sym_id(), n0 + 2, "ids follow the installs, after the two unnamed symbols");
        assert_eq!(y.sym_id(), x.sym_id() + 1);
        assert_eq!(c.install(b"x").unwrap().sym_id(), x.sym_id(), "an installed symbol keeps its id");
        // R_Newhashpjw("x") = 'x'; "foo" folds no nibble; a long name folds
        assert_eq!(x.sym_hash(), b'x' as u32);
        assert_eq!(robigo_heap::sym_hash(b"foo"), ((b'f' as u32) << 8) + ((b'o' as u32) << 4) + b'o' as u32);
        let long = c.install(b"a_rather_long_variable_name").unwrap();
        assert_eq!(long.sym_hash(), robigo_heap::sym_hash(b"a_rather_long_variable_name"));
        assert!(long.sym_hash() < 1 << 28);
        let name = long.car();
        assert_eq!(name.truelen(), long.sym_hash() as isize, "HASHVALUE on the PRINTNAME");
        assert_ne!(name.gp() & robigo_heap::gp::HASHASH, 0, "HASHASH on the PRINTNAME");
        // the name is still found in the CHARSXP cache under its marks
        assert_eq!(c.mk_char(b"a_rather_long_variable_name", 0).unwrap(), name);
    }
    // C's char is signed on x86-64: bytes >= 0x80 enter negative
    #[cfg(target_arch = "x86_64")]
    assert_eq!(robigo_heap::sym_hash("\u{e9}".as_bytes()), 0x0fff_f229);
}

/// Nodes are 48 bytes: the 4th word at +40 starts as R_NilValue, is
/// traced, and environments keep ENCLOS at +24 (council item 20).
#[test]
fn nodes_have_a_traced_fourth_word() {
    let mut c = Core::new();
    c.set_torture(false);
    assert_eq!(robigo_heap::NODE_SIZE, 48);
    let nil = c.nil();
    let n = node(&mut c, nil, nil);
    let mut roots = vec![n];
    unsafe {
        assert_eq!(n.aux(), c.nil());
        assert_eq!(*(n.0.add(robigo_heap::OFF_AUX) as *const usize), c.nil().addr(), "aux at +40");
        assert_eq!(c.nil().aux(), c.nil(), "R_NilValue's 4th word is itself");
    }
    assert_eq!(robigo_heap::OFF_SYMID, robigo_heap::OFF_AUX, "a symbol's id and hash are its 4th word");
    let v = c.alloc_vector(REALSXP, 3).unwrap();
    let addr = v.addr();
    unsafe { n.set_aux(v) };
    unsafe { assert_eq!(v.refcnt(), 1, "the store counts the reference") };
    for _ in 0..3 {
        gc(&mut c, &mut roots);
    }
    for _ in 0..10_000 {
        c.alloc_vector(REALSXP, 3).unwrap();
    }
    unsafe { assert_eq!(n.aux().addr(), addr, "the 4th word keeps its object alive") };

    let parent = unsafe { c.alloc_node(ENVSXP, nil, nil, nil) }.unwrap();
    let e = unsafe { c.alloc_node(ENVSXP, nil, nil, nil) }.unwrap();
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
    let s = c.install(b"barrier_symbol").unwrap();
    unsafe {
        assert!(s.has_flag(flags::BARRIER), "an immortal waits for its first store");
    }
    let big = c.alloc_vector(VECSXP, 100_000).unwrap();
    unsafe { c.make_immortal(big) };
    let fresh = c.alloc_vector(INTSXP, 2).unwrap();
    unsafe {
        assert!(!fresh.has_flag(flags::BARRIER), "a mortal object takes the fast path");
    }
    let v1 = c.alloc_vector(REALSXP, 1).unwrap();
    let v2 = c.alloc_vector(REALSXP, 2).unwrap();
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
        gc(&mut c, &mut ());
    }
    for _ in 0..10_000 {
        c.alloc_vector(REALSXP, 2).unwrap();
    }
    unsafe {
        assert_eq!(s.cdr().addr(), a1);
        assert_eq!((*(big.std_data() as *const Sexp).add(99_999)).addr(), a2);
        assert!(c.resolve(a1).is_some() && c.resolve(a2).is_some());
    }
}

/// The trampolines set the jump record they are given: a jump to it returns
/// 1 from that trampoline (the inner one of two nested), a normal return 0,
/// and the six-argument entry returns words, doubles and `Rcomplex`es.
#[test]
fn trampolines_and_jump() {
    use robigo_heap::unwind::{JmpBuf, Ret6, jump, robigo_heap_try_call6};
    use std::ptr::null_mut;
    /// (record, reached flag)
    type Jumps = (*mut JmpBuf, *mut bool);
    unsafe extern "C" fn nothing(_: *mut c_void) {}
    unsafe extern "C" fn jumps(d: *mut Jumps) {
        if std::hint::black_box(true) {
            // SAFETY: this frame contains nothing to drop, and the record is
            // the running trampoline's.
            unsafe { jump((*d).0) }
        }
        // SAFETY: the test's bool (never reached).
        unsafe { *(*d).1 = true }
    }
    unsafe extern "C" fn nested(outer: *mut JmpBuf) {
        let mut inner = JmpBuf::UNSET;
        let mut reached = false;
        let mut d: Jumps = (&mut inner, &mut reached);
        let a = [&raw mut d as usize, 0, 0, 0, 0, 0];
        // SAFETY: `jumps` takes one pointer; `inner` is this frame's record.
        let r = unsafe {
            robigo_heap_try_call6(&mut inner, jumps as *const c_void, a.as_ptr(), Ret6::Word, null_mut(), null_mut())
        };
        assert_eq!(r, 1);
        assert!(!reached);
        let _ = outer;
    }
    unsafe extern "C" fn add(a: usize, b: usize) -> usize {
        a + b
    }
    unsafe extern "C" fn half(a: usize, b: usize) -> f64 {
        (a + b) as f64 / 2.0
    }
    unsafe extern "C" fn cplx(a: usize, b: usize) -> robigo_heap::Rcomplex {
        robigo_heap::Rcomplex { r: a as f64, i: b as f64 }
    }
    let mut jb = JmpBuf::UNSET;
    // SAFETY: each callee takes at most six words and returns as `Ret6`
    // says; the records are this frame's and outlive the calls.
    unsafe {
        let a = [0usize; 6];
        assert_eq!(
            robigo_heap_try_call6(&mut jb, nothing as *const c_void, a.as_ptr(), Ret6::Word, null_mut(), null_mut()),
            0
        );
        let mut reached = false;
        let mut d: Jumps = (&mut jb, &mut reached);
        let a = [&raw mut d as usize, 0, 0, 0, 0, 0];
        assert_eq!(
            robigo_heap_try_call6(&mut jb, jumps as *const c_void, a.as_ptr(), Ret6::Word, null_mut(), null_mut()),
            1
        );
        assert!(!reached);
        // nested: the inner record catches, the outer call completes
        let a = [&raw mut jb as usize, 0, 0, 0, 0, 0];
        assert_eq!(
            robigo_heap_try_call6(&mut jb, nested as *const c_void, a.as_ptr(), Ret6::Word, null_mut(), null_mut()),
            0
        );
        // two word arguments of six, a word, a double, a complex back
        let a = [40usize, 3, 0, 0, 0, 0];
        let mut w = 0usize;
        assert_eq!(
            robigo_heap_try_call6(
                &mut jb,
                add as *const c_void,
                a.as_ptr(),
                Ret6::Word,
                (&raw mut w).cast(),
                null_mut()
            ),
            0
        );
        assert_eq!(w, 43);
        let mut d = 0.0f64;
        assert_eq!(
            robigo_heap_try_call6(
                &mut jb,
                half as *const c_void,
                a.as_ptr(),
                Ret6::Double,
                (&raw mut d).cast(),
                null_mut()
            ),
            0
        );
        assert_eq!(d, 21.5);
        let mut z = [0.0f64; 2];
        assert_eq!(
            robigo_heap_try_call6(
                &mut jb,
                cplx as *const c_void,
                a.as_ptr(),
                Ret6::Complex,
                (&raw mut z).cast(),
                null_mut()
            ),
            0
        );
        assert_eq!(z, [40.0, 3.0]);
    }
}

/// The Rust `JmpBuf` is the C library's `sigjmp_buf`: same size, and an
/// alignment C's divides (so a `#[repr(C)]` record embedding it lays out
/// as C's).
#[test]
fn jmpbuf_is_the_c_librarys() {
    use robigo_heap::unwind::{JMPBUF_SIZE, JmpBuf, c_jmpbuf_layout};
    let (size, align) = c_jmpbuf_layout();
    assert_eq!(size_of::<JmpBuf>(), size);
    assert_eq!(JMPBUF_SIZE, size);
    assert_eq!(align_of::<JmpBuf>(), align);
}

#[test]
fn every_object_carries_its_heaps_id() {
    let mut a = Core::new();
    let b = Core::new();
    assert_ne!(a.id(), b.id());
    // a small vector, a node, a large vector (its own chunk), a CHARSXP and
    // the immortal constants made with the heap (allocation never collects)
    let small = a.alloc_vector(INTSXP, 3).unwrap();
    let big = a.alloc_vector(REALSXP, 100_000).unwrap();
    let nil = a.nil();
    let node = unsafe { a.alloc_node(LISTSXP, nil, nil, nil) }.unwrap();
    let s = a.mk_char(b"owner", 0).unwrap();
    let sym = a.install(b"sym").unwrap();
    for x in [small, big, node, s, a.nil(), a.na_string(), sym] {
        assert_eq!(unsafe { x.heap_id() }, a.id());
        assert_ne!(unsafe { x.heap_id() }, b.id());
    }
    let mut b = b;
    let y = b.alloc_vector(INTSXP, 1).unwrap();
    assert_eq!(unsafe { y.heap_id() }, b.id());
}
