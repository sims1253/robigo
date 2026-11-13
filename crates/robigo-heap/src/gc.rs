//! The collector and the allocation entry points: a non-moving
//! mark-region heap (blocks, size classes: `heap.rs`) with precise roots
//! (the embedder's [`Trace`] state, the precious set, weak references) and
//! a conservative scan of the C segments of the machine stack. Ported from
//! the bake-off's `mm-gc` (`experiments/memory`), which measured this
//! design against reference counting (`docs/ledger/memory.md`).
//!
//! **Allocation never collects** (docs/g3.md §3). The poll-only path
//! counts GNU R's heap sizes and, when the trigger is crossed, sets
//! [`GC_DUE`] in the poll word; it refuses ([`NeedGc`]) only what it cannot
//! serve without collecting (a limit, a refused mapping). Collections run
//! when the embedder asks for one, with its roots: at a safepoint
//! ([`Core::safepoint`]), before an allocation under `&mut Cx`
//! ([`Core::alloc_rooted`], [`Core::make_room`]), or explicitly
//! ([`Core::collect`]).
//!
//! **No Rust frame is ever scanned.** The conservative scan reads only C
//! segments: the frames of package code between a native entry, which
//! opens one (`robigo_heap_try_call65`), and the export it called, which
//! closes it ([`Core::close_scan_segment`]) before it may collect.

use crate::heap::{
    BLOCK, Block, CLASS_NONE, DATA_OFF, Heap, MAX_MEDIUM, OWNER_EMPTY, OWNER_NONE, Remembered, SIZES, START_WORDS,
    remembered_of,
};
use crate::heapsize::{HeapSizes, vec_cells};
use crate::trace::{Trace, Visitor};
use crate::types::*;
use crate::{EXTDATA_SIZE, NODE_SIZE, OFF_DATA, Sexp, flags, gp, hash_bytes, is_vector_type};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// Free slots or bump space left in a block (the sweep re-lists such
/// blocks: it resets the allocation cursors, and a block left off every
/// list would keep its unused space until all its objects died).
///
/// # Safety
/// `b` must be a block of this heap assigned to a size class.
unsafe fn block_has_room(b: *mut Block) -> bool {
    // SAFETY: the caller's contract: an assigned header, a valid class index.
    unsafe { !(*b).freelist.is_null() || (*b).bump_next + (SIZES[(*b).class as usize] / 16) as u32 <= (*b).granules }
}

/// `alloc_vector` of a type without elements: tracing the result would read
/// its length word as a car.
#[cold]
#[inline(never)]
#[track_caller]
fn not_a_vector_type(t: u8) -> ! {
    panic!("robigo: SEXPTYPE {t} is not a vector type")
}

/// A vector's allocation: its bytes (header included, rounded to 16) and
/// GNU R's count of its data in VECRECs, or None past the address space.
fn vector_size(t: u8, n: usize) -> Option<(usize, usize)> {
    let bytes = n.checked_mul(crate::elt_size(t))?.checked_add(OFF_DATA + 15)? / 16 * 16;
    Some((bytes, vec_cells(t, n)))
}

/// The VECRECs of a request too large to compute exactly: for GNU R's
/// "cannot allocate vector of size" message.
fn vecrecs_saturating(t: u8, n: usize) -> usize {
    n.saturating_mul(crate::elt_size(t)).div_ceil(8)
}

/// What a refused request needs from the vector heap, in VECRECs.
fn request_cells(req: NeedGc) -> Result<usize, AllocError> {
    let (t, n) = (req.sexptype(), req.length());
    if !is_vector_type(t) {
        return Ok(0);
    }
    // (a CHARSXP's cells count its NUL: vec_cells adds it; a saturated
    // length is past the address space)
    vector_size(t, n)
        .filter(|_| n as u64 != NeedGc::LENGTH)
        .map(|(_, cells)| cells)
        .ok_or(AllocError::CannotAllocate { vecrecs: vecrecs_saturating(t, n) })
}

/// Releases an external payload when its object dies.
pub type ReleaseFn = unsafe extern "C" fn(usize);

// Stored at +48 of EXTDATA objects, which C can see: one word, and
// null for None (a guarantee of `Option<fn>`, so capi's disarming write of 0
// is `None`).
const _: () = assert!(std::mem::size_of::<Option<ReleaseFn>>() == std::mem::size_of::<usize>());

/// The poll word's bit the allocator sets when a collection is due (docs/g3.md
/// §3 item 1: `cx.poll |= GC_DUE`). The heap owns this bit only: the
/// embedder may keep other reasons to stop in the same word
/// ([`Core::set_poll_word`]).
pub const GC_DUE: u32 = 1;

/// The size from which an allocation under `&Cx` should go through
/// [`Core::alloc_rooted`] instead (docs/g3.md §3 item 3; measured, starting
/// at 1 MiB): a `&Cx` request this large made while a collection is due is
/// counted ([`GcStats::large_while_due`]), and fails CI.
pub const LARGE: usize = 1 << 20;

/// E4's overshoot bound (docs/g3.md §15.3): the bytes allocated between a
/// collection coming due and the collection that serves it stay below
/// max(32 MiB, 10% of the live heap).
pub fn overshoot_bound(live_bytes: u64) -> u64 {
    (32u64 << 20).max(live_bytes / 10)
}

/// An allocation the poll-only path refused: it could not be served
/// without collecting first, or at all (a vector or node limit, a refused
/// mapping, a size past the address space). The request, so that the
/// caller can retry exactly it through `&mut Cx` ([`Core::make_room`],
/// [`Core::alloc_rooted`]), where the answer is a collection or GNU R's
/// error ([`AllocError`]); docs/g3.md §3 item 5.
///
/// One word, so that `Result<Sexp, NeedGc>` comes back in two registers:
/// the SEXPTYPE in the top byte, the length below. A length of 2^56 or
/// more (no R vector has one: `R_XLEN_T_MAX` is 2^52) is kept as 2^56 - 1,
/// past the address space either way.
#[must_use = "an allocation failed: retry it through &mut Cx"]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NeedGc(u64);

impl NeedGc {
    const LENGTH: u64 = (1 << 56) - 1;

    /// A request for `length` elements of SEXPTYPE `sexptype`. For a node,
    /// `length` is 1; for an external-data vector, which GNU R counts as a
    /// node, 0; for a CHARSXP, its bytes without the NUL.
    #[inline(always)]
    pub const fn new(sexptype: u8, length: usize) -> NeedGc {
        let n = if length as u64 > Self::LENGTH { Self::LENGTH } else { length as u64 };
        NeedGc((sexptype as u64) << 56 | n)
    }

    /// The SEXPTYPE requested.
    pub const fn sexptype(self) -> u8 {
        (self.0 >> 56) as u8
    }

    /// The number of elements requested (2^56 - 1 for any larger number).
    pub const fn length(self) -> usize {
        (self.0 & Self::LENGTH) as usize
    }
}

impl std::fmt::Debug for NeedGc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NeedGc").field("sexptype", &self.sexptype()).field("length", &self.length()).finish()
    }
}

/// An allocation that failed after collecting: GNU R's errors (memory.c),
/// which the embedder raises with no call (`errorcall(R_NilValue, ...)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// The vector heap limit (`R_MaxVSize`, in bytes; None: unlimited).
    VectorMemory { limit: Option<usize> },
    /// The node limit (`R_MaxNSize`; None: unlimited).
    ConsMemory { limit: Option<usize> },
    /// The system refused the memory, or the size does not fit in the
    /// address space: the request's vector size in 8-byte VECRECs.
    CannotAllocate { vecrecs: usize },
}

impl AllocError {
    /// GNU R's message, verbatim.
    // R: memory.c:mem_err_heap, mem_err_cons, allocVector3 (`! success`)
    pub fn message(&self) -> String {
        match *self {
            AllocError::VectorMemory { limit: None } => "vector memory exhausted".to_string(),
            AllocError::VectorMemory { limit: Some(bytes) } => {
                let mut l = bytes as f64 / 1024.0;
                let mut unit = "Kb";
                if l > 1024.0 * 1024.0 {
                    l /= 1024.0 * 1024.0;
                    unit = "Gb";
                } else if l > 1024.0 {
                    l /= 1024.0;
                    unit = "Mb";
                }
                format!("vector memory limit of {l:.1} {unit} reached, see mem.maxVSize()")
            }
            AllocError::ConsMemory { limit: None } => "cons memory exhausted".to_string(),
            AllocError::ConsMemory { limit: Some(n) } => {
                format!("cons memory limit of {n} nodes reached, see mem.maxNSize()")
            }
            AllocError::CannotAllocate { vecrecs } => {
                let dsize = vecrecs as f64 * 8.0 / 1024.0;
                if dsize > 1024.0 * 1024.0 {
                    format!("cannot allocate vector of size {:.1} Gb", dsize / 1024.0 / 1024.0)
                } else if dsize > 1024.0 {
                    format!("cannot allocate vector of size {:.1} Mb", dsize / 1024.0)
                } else {
                    format!("cannot allocate vector of size {dsize:.0} Kb")
                }
            }
        }
    }
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for AllocError {}

/// Where a collection was entered: the safepoint kinds of docs/g3.md §3
/// item 2, counted per kind by `ROBIGO_GC_STATS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Safepoint {
    /// An R entry: eval, closure application, forcing a promise, VM entry,
    /// a native call, the finalizer runner.
    Entry,
    /// A VM back-edge or call (merged with the interrupt poll).
    Loop,
    /// A builtin's return.
    Return,
    /// `cx.poll_alloc` in a Rust loop.
    PollAlloc,
    /// [`Core::alloc_rooted`]: a request that collects first.
    Rooted,
    /// [`Core::make_room`]: the retry of a refused `&Cx` allocation.
    Retry,
    /// An allocating C API export, at its entry.
    Export,
    /// `gc()`, `R_gc()`: [`Core::collect`].
    Explicit,
}

impl Safepoint {
    pub const COUNT: usize = 8;
    pub const ALL: [Safepoint; Safepoint::COUNT] = [
        Safepoint::Entry,
        Safepoint::Loop,
        Safepoint::Return,
        Safepoint::PollAlloc,
        Safepoint::Rooted,
        Safepoint::Retry,
        Safepoint::Export,
        Safepoint::Explicit,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Safepoint::Entry => "entry",
            Safepoint::Loop => "loop",
            Safepoint::Return => "return",
            Safepoint::PollAlloc => "poll_alloc",
            Safepoint::Rooted => "alloc_rooted",
            Safepoint::Retry => "retry",
            Safepoint::Export => "export",
            Safepoint::Explicit => "explicit",
        }
    }
}

/// What `ROBIGO_GC_STATS` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatsMode {
    /// Unset or `0`: only the counters every collection keeps.
    Off,
    /// `1`: also count allocations by SEXPTYPE (every allocation then takes
    /// the slow path), and print the report when the heap is dropped.
    Report,
    /// `check`: as `Report`, and a `&Cx` request of [`LARGE`] bytes or more
    /// made while a collection is due panics where it is made (CI).
    Check,
}

impl StatsMode {
    fn from_env() -> StatsMode {
        match std::env::var("ROBIGO_GC_STATS").as_deref() {
            Ok("check") => StatsMode::Check,
            Ok("") | Ok("0") | Err(_) => StatsMode::Off,
            Ok(_) => StatsMode::Report,
        }
    }
}

/// Counters the ledger records.
#[derive(Debug, Clone, Default)]
pub struct GcStats {
    /// Objects allocated (only while [`StatsMode`] is not `Off`).
    pub allocs: u64,
    pub bytes_allocated: u64,
    pub collections: u64,
    pub frees: u64,
    pub live_bytes: u64,
    pub live_objects: u64,
    pub collect_total_ns: u64,
    pub collect_max_pause_ns: u64,
    /// Collections by where they were entered ([`Safepoint`] as index).
    pub collections_by_kind: [u64; Safepoint::COUNT],
    /// The largest overshoot: bytes allocated from the allocation that set
    /// [`GC_DUE`] (included) to the collection that served it (E4), and the
    /// live bytes that collection left.
    pub max_overshoot: u64,
    pub max_overshoot_live: u64,
    /// The sum of all overshoots, and the collections whose overshoot
    /// exceeded [`overshoot_bound`].
    pub total_overshoot: u64,
    pub overshoot_over_bound: u64,
    /// `&Cx` requests of [`LARGE`] bytes or more made while a collection
    /// was due (docs/g3.md §3 item 3: a CI failure), and the largest.
    pub large_while_due: u64,
    pub largest_while_due: u64,
    /// Objects and bytes allocated by SEXPTYPE (only while [`StatsMode`] is
    /// not `Off`). Bytes are the request rounded to 16, as `bytes_allocated`.
    pub by_type: [(u64, u64); 32],
}

/// C's `atoi`: optional blanks and sign, then digits; 0 if none.
fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        n = (n * 10 + (b - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    (if neg { -n } else { n }).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// The torture counters the environment asks for: `(wait, gap)`.
// R: memory.c:init_gctorture (R_GCTORTURE, R_GCTORTURE_WAIT; robigo's
// ROBIGO_GCTORTURE is read when R_GCTORTURE is not set)
fn torture_from_env() -> (i32, i32) {
    let Some(arg) = std::env::var("R_GCTORTURE").or_else(|_| std::env::var("ROBIGO_GCTORTURE")).ok() else {
        return (0, 0);
    };
    let gap = atoi(&arg);
    if gap <= 0 {
        return (0, 0);
    }
    let wait = std::env::var("R_GCTORTURE_WAIT").map(|w| atoi(&w)).unwrap_or(0);
    (if wait > 0 { wait } else { gap }, gap)
}

/// One heap and its tables. Single-threaded, like R's values.
pub struct Core {
    heap: Heap,
    mark_stack: Vec<Sexp>,
    /// the collection's number: large objects are marked by storing it in
    /// their chunk's header (no clearing between collections)
    epoch: u64,
    /// bytes allocated since the last collection (for `stats`)
    alloc_since_gc: usize,
    /// when to collect: GNU R's heap sizes and what is in use (heapsize.rs)
    sizes: HeapSizes,
    /// cached CHARSXPs marked by this collection, and immortal ones: when
    /// they add up to the table's size nothing in it died, and the prune
    /// (a cache miss per entry: 1.3 s for 10M strings) is skipped
    marked_chars: usize,
    immortal_chars: usize,
    /// Positive: safepoints do not collect, and a collection due stays due
    /// (GNU R's `R_GCEnabled` off: `R_gc_internal` postpones). Allocation
    /// never collects either way; [`Core::collect`] still does.
    pub gc_inhibit: u32,
    /// a collection is running: a nested one panics
    collecting: bool,
    /// A collection is due: the poll-only path crossed the trigger, or
    /// gctorture's countdown ran out. Mirrored in the poll word's
    /// [`GC_DUE`]; cleared by the collection that serves it.
    due: bool,
    /// `alloc_since_gc` when `due` was set: the overshoot starts there
    due_at: usize,
    /// the embedder's poll word ([`Core::set_poll_word`]), or None for
    /// `own_poll`
    poll: Option<NonNull<AtomicU32>>,
    own_poll: AtomicU32,
    /// `ROBIGO_GC_STATS`, read when the core is made
    stats_mode: StatsMode,
    /// the bounds of the thread the core is bound to (the evaluator's)
    stack_top: usize,
    stack_lo: usize,
    /// The top of the open C segment: the frames of package code that a
    /// native entry called (`robigo_heap_try_call65` sets it at the
    /// routine's stack arguments), up to the export it calls in turn,
    /// which closes the segment before it may collect. 0: no C code runs,
    /// or its segment is closed. See [`Core::close_scan_segment`].
    scan_top: usize,
    /// The closed C segments, innermost first: C code that called an
    /// export (see [`Core::close_scan_segment`]). The nodes live in the
    /// exports' frames.
    scan_chain: *const ScanSegment,
    /// gctorture (memory.c's gc_force_wait, gc_force_gap): while
    /// `force_wait > 0`, every allocation counts it down and collects when
    /// it reaches 0, then waits `force_gap` allocations for the next one;
    /// freed slots are poisoned while `force_gap > 0` (R_GCTORTURE)
    force_wait: i32,
    force_gap: i32,
    pub stats: GcStats,

    // the immortal constants
    nil: Sexp,
    na_string: Sexp,
    blank_string: Sexp,
    unbound: Sexp,
    missing_arg: Sexp,

    /// the CHARSXP cache: weak, pruned after marking
    chars: crate::chars::CharTable,
    /// symbols by name (immortal)
    syms: HashMap<Box<[u8]>, Sexp, BuildFx>,

    /// R_PreserveObject's multiset, as GNU R's precious list: a stack,
    /// released from the top (O(1) for the usual LIFO churn)
    precious: Vec<Sexp>,
    /// Experiment A (docs/g3.md §15.1): C words only pin memory; weak
    /// references and finalizers are decided on the precise marks alone.
    /// `ROBIGO_GC_PIN_ONLY=1`, read when the core is made. H2 step 6 makes
    /// this order the only one once M0's adoption rule has passed
    /// ([`Core::finish_collection`]).
    pin_only: bool,
    /// weak references (WEAKREFSXP) not yet finalized
    weak_refs: Vec<Sexp>,
    /// weak references whose key died, waiting for their finalizer
    finalize_queue: Vec<Sexp>,
    /// release callbacks of objects found dead by the last sweep
    releases: Vec<(ReleaseFn, usize)>,
    /// The store barrier's log ([`Sexp::store`]): immortal objects
    /// something was stored into (a symbol's value, attributes set on a
    /// shared constant), traced on every collection. Each is logged once:
    /// the log clears the object's [`flags::BARRIER`]. Its own allocation,
    /// which every block header points at ([`crate::heap::Remembered`]).
    remembered: Box<Remembered>,
}

/// The store barrier's slow path, which then does the store: `x` has
/// [`flags::BARRIER`] set. Logs
/// `x` in its heap's remembered set and clears the bit, so `x` is logged
/// once. A generational collector adds its old-to-young check here
/// (GNU R's CHECK_OLD_TO_NEW).
///
/// # Safety
/// `x` must be a live object of a live heap, on that heap's thread, and
/// no reference into the remembered set may be live (only `collect` takes
/// one, and nothing stores while it runs).
// R: memory.c:CHECK_OLD_TO_NEW (old_to_new)
#[cold]
#[inline(never)]
pub(crate) unsafe fn store_slow(x: Sexp, slot: *mut Sexp, v: Sexp) {
    // SAFETY: the caller's contract (`Sexp::store`'s).
    unsafe {
        barrier_slow(x);
        x.store_counted(slot, v);
    }
}

/// # Safety
/// As for [`store_slow`].
#[inline(always)]
unsafe fn barrier_slow(x: Sexp) {
    // SAFETY: the caller's contract: `x` is live, so its header and its
    // block's header are readable, and the remembered set is not borrowed.
    unsafe {
        x.set_flag(flags::BARRIER, false);
        // A CHARSXP has nothing to trace, and must not be: `trace` would
        // count it in `marked_chars` on top of `immortal_chars`, and with
        // the sum inflated `prune_chars` could skip a table with dead
        // entries.
        if x.typ() == CHARSXP {
            return;
        }
        let r = remembered_of(x.0);
        (*(*r).0.get()).push(x);
    }
}

#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);
impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0 = (self.0.rotate_left(5) ^ hash_bytes(bytes) as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u32(&mut self, w: u32) {
        self.0 = (self.0.rotate_left(5) ^ w as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_usize(&mut self, w: usize) {
        self.0 = (self.0.rotate_left(5) ^ (w as u64 >> 4)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}
pub type BuildFx = std::hash::BuildHasherDefault<FxHasher>;

/// Miri cannot ask for the thread's stack, and runs no conservative scan
/// (its users, robigo-rtcore's tests, never collect): an empty stack.
#[cfg(miri)]
fn thread_stack() -> (usize, usize) {
    (0, 0)
}

/// The calling thread's stack: (lowest address, highest address).
#[cfg(all(target_vendor = "apple", not(miri)))]
fn thread_stack() -> (usize, usize) {
    // SAFETY: both only read the attributes of the calling thread, and the
    // first returns the high end, as stacks grow down.
    unsafe {
        let thread = libc::pthread_self();
        let high = libc::pthread_get_stackaddr_np(thread) as usize;
        (high - libc::pthread_get_stacksize_np(thread), high)
    }
}

/// The calling thread's stack: (lowest address, highest address).
#[cfg(all(not(target_vendor = "apple"), not(miri)))]
fn thread_stack() -> (usize, usize) {
    // SAFETY: an all-zero pthread_attr_t is only storage for
    // pthread_getattr_np to initialize; the attribute calls below read it
    // only after that succeeded, and write into locals that outlive them.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            no_stack_bounds();
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let got = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if got != 0 {
            no_stack_bounds();
        }
        (addr as usize, addr as usize + size)
    }
}

/// Without its bounds the stack cannot be scanned, and an object held only
/// by a C local would be freed: stop rather than run without the scan.
#[cfg(all(not(target_vendor = "apple"), not(miri)))]
#[cold]
fn no_stack_bounds() -> ! {
    eprintln!("robigo: cannot determine the bounds of the thread's stack");
    std::process::abort();
}

#[inline(always)]
fn stack_pointer() -> usize {
    let marker = 0usize;
    std::hint::black_box(&marker) as *const usize as usize
}

/// Can a word one past the end of `p`'s allocation be a C loop bound over
/// `p`'s data (`REAL(x) + n`)? Only if `p` is a standard vector with its
/// data inline. One at the end of a node, an environment or an external
/// pointer is a pointer to the next slot's object (or a stale one), and it
/// kept a dead neighbour's finalizer from running (capi
/// `weak-keys-collections`, ledger 2026-11-11). The caller found `p`
/// through `w - 1` with `w` on a granule boundary, so `w` is either inside
/// `p`'s slot (then `resolve(w)` marked `p` already) or the end of `p`'s
/// slot or large chunk: the allocation's end, whatever `p`'s length says
/// now (`SETLENGTH` shortens a vector in place, and C may keep the end of
/// the buffer it allocated).
///
/// # Safety
/// `p` must be an allocated object (what `resolve` returns).
#[inline]
unsafe fn has_inline_data(p: Sexp) -> bool {
    // SAFETY: `p` is allocated (the caller's contract), so its header is
    // readable.
    unsafe { is_vector_type(p.typ()) && !p.has_flag(flags::ALTREP | flags::EXTDATA) }
}

/// A closed C segment of the conservative scan: `[lo, hi)` of the machine
/// stack, the frames of package C code between a native entry (`hi`) and
/// the export it called (`lo`), and, scanned with it, the callee-saved
/// registers C left in place when the export's entry spilled them into the
/// node instead of pushing them into `[lo, hi)` (see
/// [`Core::close_scan_segment`]). Linked from the innermost; the node lives
/// in the export's frame.
#[repr(C)]
pub struct ScanSegment {
    lo: usize,
    hi: usize,
    prev: *const ScanSegment,
    regs: [usize; 12],
}

impl ScanSegment {
    pub const fn new() -> Self {
        ScanSegment { lo: 0, hi: 0, prev: std::ptr::null(), regs: [0; 12] }
    }

    /// Store the callee-saved registers into the node, for an export whose
    /// machine entry did not push them below its return address: first
    /// thing in the export, before it changes one (registers its prologue
    /// already saved are in its frame, above the node).
    #[inline(always)]
    pub fn spill_registers(&mut self) {
        spill_callee_saved(&mut self.regs);
    }
}

impl Default for ScanSegment {
    fn default() -> Self {
        Self::new()
    }
}

/// The callee-saved registers, stored into `buf`: called first thing in an
/// export, it keeps every value the C frames have in such a register, which
/// later frames below would otherwise save out of the scan's reach.
#[inline(always)]
fn spill_callee_saved(buf: &mut [usize; 12]) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: stores six registers into the twelve words `buf` points at;
    // reads no memory, changes no register or flag the compiler relies on.
    unsafe {
        std::arch::asm!(
            "mov [{b}], rbx",
            "mov [{b} + 8], rbp",
            "mov [{b} + 16], r12",
            "mov [{b} + 24], r13",
            "mov [{b} + 32], r14",
            "mov [{b} + 40], r15",
            b = in(reg) buf.as_mut_ptr(),
            options(nostack, preserves_flags),
        );
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: as above, eleven registers (x19-x29) into `buf`.
    unsafe {
        std::arch::asm!(
            "stp x19, x20, [{b}]",
            "stp x21, x22, [{b}, #16]",
            "stp x23, x24, [{b}, #32]",
            "stp x25, x26, [{b}, #48]",
            "stp x27, x28, [{b}, #64]",
            "str x29, [{b}, #80]",
            b = in(reg) buf.as_mut_ptr(),
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("spill_callee_saved: name this architecture's callee-saved registers");
}

/// The message of a failure to allocate R's constants (startup: the heap
/// is empty and unlimited but for `R_MAX_VSIZE`, whose minimum fits them).
const CONSTANTS: &str = "robigo: cannot allocate R's constants";

impl Core {
    /// A new heap. The calling thread's stack is the one whose C segments
    /// are scanned.
    pub fn new() -> Box<Core> {
        let (stack_lo, stack_top) = thread_stack();
        let mut c = Box::new(Core {
            heap: Heap::new(),
            mark_stack: Vec::new(),
            epoch: 0,
            alloc_since_gc: 0,
            sizes: HeapSizes::new(),
            marked_chars: 0,
            immortal_chars: 0,
            gc_inhibit: 0,
            collecting: false,
            due: false,
            due_at: 0,
            poll: None,
            own_poll: AtomicU32::new(0),
            stats_mode: StatsMode::from_env(),
            stack_top,
            stack_lo,
            scan_top: 0,
            scan_chain: std::ptr::null(),
            force_wait: 0,
            force_gap: 0,
            stats: GcStats::default(),
            nil: Sexp::NULL,
            na_string: Sexp::NULL,
            blank_string: Sexp::NULL,
            unbound: Sexp::NULL,
            missing_arg: Sexp::NULL,
            chars: crate::chars::CharTable::new(),
            syms: HashMap::default(),
            precious: Vec::new(),
            pin_only: std::env::var_os("ROBIGO_GC_PIN_ONLY").is_some_and(|v| v == "1"),
            weak_refs: Vec::new(),
            finalize_queue: Vec::new(),
            releases: Vec::new(),
            remembered: Box::new(Remembered(Default::default())),
        });
        // every block and large chunk header points at the log, before the
        // first allocation makes one
        c.heap.remembered = &*c.remembered as *const Remembered as *mut Remembered;
        // SAFETY: every object here is fresh from this heap, node-sized where
        // node fields are written, and flagged immortal before any
        // collection, so none of them can be swept.
        unsafe {
            // R_NilValue: a node whose fields point at itself (memory.c InitMemory)
            let nil = c.alloc_object(NILSXP, 1, NODE_SIZE, 0).expect(CONSTANTS);
            nil.set_attrib_raw(nil);
            nil.set_car_raw(nil);
            nil.set_cdr_raw(nil);
            nil.set_tag_raw(nil);
            nil.set_sexp_at(crate::OFF_AUX, nil);
            c.nil = nil;
            c.make_immortal(nil);
            nil.set_attrib_raw(nil);
            // NA_STRING is not in the cache (names.c: mkChar("NA") is a different object)
            let na = c.alloc_char(b"NA", 0, hash_bytes(b"NA")).expect(CONSTANTS);
            c.make_immortal(na);
            c.na_string = na;
            let blank = c.mk_char(b"", 0).expect(CONSTANTS);
            c.make_immortal(blank);
            c.blank_string = blank;
            // R_UnboundValue and R_MissingArg are symbols with empty names
            // that are not in the symbol table (names.c InitNames)
            c.unbound = c.new_symbol(blank).expect(CONSTANTS);
            c.missing_arg = c.new_symbol(blank).expect(CONSTANTS);
        }
        // like InitMemory: once the constants exist
        (c.force_wait, c.force_gap) = torture_from_env();
        c.retrigger();
        c
    }

    /// Flag `x` immortal: never marked, never freed. Only for objects no
    /// other object can be traced through to anything mortal.
    ///
    /// # Safety
    /// `x` must be a live object of this heap. From here on the collector
    /// does not trace `x`: whatever it points at now must be immortal, or be
    /// kept alive some other way, until a store into `x` passes
    /// the store barrier ([`Sexp::store`]), which makes `x` a root. An object flagged
    /// without this function is missing from its block's count, and the
    /// sweep miscounts.
    pub unsafe fn make_immortal(&mut self, x: Sexp) {
        // SAFETY: `x` is live, so its header can be read and written. Its
        // block's address is in a chunk, and a large chunk's first word
        // (cleared when allocated) cannot equal the magic number.
        unsafe {
            if x.has_flag(flags::IMMORTAL) {
                return;
            }
            // the first store into it is logged (`barrier_slow`)
            x.set_flag(flags::IMMORTAL | flags::BARRIER, true);
            if x.typ() == CHARSXP && x.gp() & gp::CACHED != 0 {
                self.immortal_chars += 1;
            }
            let b = self.heap.block_of(x.0);
            if (*b).magic == 0x524f4249474f4241 {
                (*b).immortals += 1;
            }
            // large objects: the sweeper keeps them by the flag
        }
    }

    // ---- the immortal constants and the embedder's hooks ----------------

    /// `R_NilValue`.
    #[inline(always)]
    pub fn nil(&self) -> Sexp {
        self.nil
    }
    /// `NA_STRING`.
    #[inline(always)]
    pub fn na_string(&self) -> Sexp {
        self.na_string
    }
    /// `R_BlankString`.
    #[inline(always)]
    pub fn blank_string(&self) -> Sexp {
        self.blank_string
    }
    /// `R_UnboundValue`.
    #[inline(always)]
    pub fn unbound(&self) -> Sexp {
        self.unbound
    }
    /// `R_MissingArg`.
    #[inline(always)]
    pub fn missing_arg(&self) -> Sexp {
        self.missing_arg
    }

    // ---- the poll word ------------------------------------------------

    /// The word whose [`GC_DUE`] bit says a collection is due: a safepoint
    /// is `if poll != 0 { ... }`, one load and one branch.
    pub fn poll_word(&self) -> &AtomicU32 {
        match self.poll {
            // SAFETY: `set_poll_word`'s contract keeps it valid.
            Some(w) => unsafe { w.as_ref() },
            None => &self.own_poll,
        }
    }

    /// Set [`GC_DUE`] in `word` from now on instead of in the heap's own
    /// word: an embedder that polls one word for several reasons
    /// (interrupts, pending finalizers) keeps it where its safepoints load
    /// it. The bit's current state moves to `word`; the other bits of
    /// `word` are the embedder's, and the heap only sets and clears its own.
    ///
    /// # Safety
    /// `word` must stay valid, at this address, until the core is dropped
    /// or another word replaces it.
    pub unsafe fn set_poll_word(&mut self, word: NonNull<AtomicU32>) {
        let due = self.poll_word().fetch_and(!GC_DUE, Ordering::Relaxed) & GC_DUE;
        self.poll = Some(word);
        self.poll_word().fetch_or(due, Ordering::Relaxed);
    }

    /// Whether a collection is due (the poll-only path crossed the trigger,
    /// or gctorture's countdown ran out), served at the next safepoint.
    #[inline(always)]
    pub fn gc_due(&self) -> bool {
        self.due
    }

    // ---- allocation ---------------------------------------------------

    /// The collection trigger's state in GNU R's units.
    pub fn heap_sizes(&self) -> HeapReport {
        HeapReport {
            n_size: self.sizes.n_size,
            v_size: self.sizes.v_size,
            nodes_in_use: self.sizes.nodes_in_use,
            vecrecs_in_use: self.sizes.valloc,
        }
    }

    /// `mem.maxVSize()`: the vector heap limit in bytes, None if unlimited.
    pub fn max_vsize(&self) -> Option<usize> {
        self.sizes.max_vsize()
    }

    /// Set the vector heap limit in bytes (None: unlimited). Refused (false)
    /// below the current vector heap size, as `R_SetMaxVSize`: the caller
    /// warns "a limit lower than current usage, so ignored".
    pub fn set_max_vsize(&mut self, bytes: Option<usize>) -> bool {
        let ok = self.sizes.set_max_vsize(bytes);
        self.retrigger();
        ok
    }

    /// `mem.maxNSize()`: the node limit, None if unlimited.
    pub fn max_nsize(&self) -> Option<usize> {
        self.sizes.max_nsize()
    }

    /// Set the node limit (None: unlimited). Refused (false) below the
    /// current node heap size, as `R_SetMaxNSize`.
    pub fn set_max_nsize(&mut self, n: Option<usize>) -> bool {
        let ok = self.sizes.set_max_nsize(n);
        self.retrigger();
        ok
    }

    /// Point the poll-only path's fast test at what the state needs: every
    /// allocation to the slow path under gctorture or the allocation
    /// statistics, the limits while a collection is due (only a refusal is
    /// left to find), else GNU R's trigger.
    fn retrigger(&mut self) {
        let s = &mut self.sizes;
        if self.force_wait > 0 || self.stats_mode != StatsMode::Off {
            (s.n_trigger, s.v_trigger) = (0, 0);
        } else if self.due {
            (s.n_trigger, s.v_trigger) = (s.max_n_size, s.max_v_size);
        } else {
            (s.n_trigger, s.v_trigger) = (s.n_size, s.v_size);
        }
    }

    /// A collection is due from here: set the poll word's bit and start
    /// counting the overshoot (from the allocation that crossed, which is
    /// not counted yet).
    #[cold]
    fn set_due(&mut self) {
        self.due = true;
        self.due_at = self.alloc_since_gc;
        self.poll_word().fetch_or(GC_DUE, Ordering::Relaxed);
        self.retrigger();
    }

    /// The poll-only path's slow branch, for an object of `bytes` bytes
    /// counted as `cells` VECRECs: refuse what would pass a limit (counting
    /// the garbage, which only a collection can tell apart), count by type
    /// for the statistics, run gctorture's countdown, and set the due bit
    /// when GNU R would collect before this allocation. Never collects.
    // R: memory.c:allocVector3 / allocSExp: `if (FORCE_GC || NO_FREE_NODES()
    // || VHEAP_FREE() < alloc_size) R_gc_internal(alloc_size)`, with
    // R_gc_internal's postponing branch (`!R_GCEnabled`) as the only one:
    // the collection waits for a safepoint
    #[cold]
    #[inline(never)]
    fn poll_slow(&mut self, t: u8, n: usize, bytes: usize, cells: usize) -> Result<(), NeedGc> {
        #[cfg(debug_assertions)]
        let collections = self.stats.collections;
        // (with collections inhibited, GNU R checks the vector limit only:
        // R_gc_internal's `!R_GCEnabled` branch grows R_NSize unchecked)
        let within =
            if self.gc_inhibit > 0 { self.sizes.within_vector_limit(cells) } else { self.sizes.within_limits(cells) };
        if !within {
            return Err(NeedGc::new(t, n));
        }
        if self.stats_mode != StatsMode::Off {
            self.note_alloc(t, bytes);
        }
        // (gctorture's countdown waits while a collection is due, as
        // FORCE_GC's `gc_pending ||` does)
        if !self.due && ((self.force_wait > 0 && self.force_gc()) || self.sizes.due(cells)) {
            self.set_due();
        }
        // docs/g3.md §3 item 1, checked where gctorture would have
        // collected: the allocation path never does
        #[cfg(debug_assertions)]
        debug_assert_eq!(self.stats.collections, collections, "robigo: an allocation collected");
        Ok(())
    }

    /// Count an allocation by type for the statistics.
    #[cold]
    fn note_alloc(&mut self, t: u8, bytes: usize) {
        self.stats.allocs += 1;
        let e = &mut self.stats.by_type[t as usize & 31];
        e.0 += 1;
        e.1 += bytes as u64;
    }

    /// Count a new object of `cells` VECRECs in the heap sizes.
    #[inline(always)]
    fn count(&mut self, bytes: usize, cells: usize) {
        self.alloc_since_gc += bytes;
        self.sizes.nodes_in_use += 1;
        self.sizes.valloc += cells;
    }

    /// Write a fresh object's header: type `t`, no flags, attributes
    /// R_NilValue.
    ///
    /// # Safety
    /// `p` must be fresh from the heap, with at least 16 bytes.
    #[inline(always)]
    unsafe fn init_header(&self, p: *mut u8, t: u8) -> Sexp {
        // SAFETY: the caller's contract; writing +0 as a word clears the
        // flags, gp and counts.
        unsafe {
            *(p as *mut u64) = t as u64;
            *((p as *mut *mut u8).add(1)) = self.nil.0;
        }
        Sexp(p)
    }

    /// A raw object of `size` bytes, at most `MAX_MEDIUM`, with a fresh
    /// header, whose vector data GNU R would count as `cells` VECRECs
    /// ([`crate::heapsize::vec_cells`]; 0 for nodes): the poll-only path
    /// for the request `t`/`n`. Never collects.
    #[inline(always)]
    pub(crate) fn alloc_object(&mut self, t: u8, n: usize, size: usize, cells: usize) -> Result<Sexp, NeedGc> {
        debug_assert!(size <= MAX_MEDIUM);
        if self.sizes.over_trigger(cells) {
            self.poll_slow(t, n, size, cells)?;
        }
        let size = size.max(16);
        let p = self.heap.alloc(size);
        // SAFETY: `p` is fresh, with at least 16 bytes.
        let x = unsafe { self.init_header(p, t) };
        self.count(size, cells);
        Ok(x)
    }

    /// As [`Core::alloc_object`] for an object of any size: past
    /// `MAX_MEDIUM` it has a chunk of its own, which the system may refuse.
    /// For `&Cx` requests only: a large one is checked against `GC_DUE`.
    #[inline]
    fn alloc_sized(&mut self, t: u8, n: usize, size: usize, cells: usize) -> Result<Sexp, NeedGc> {
        if size <= MAX_MEDIUM { self.alloc_object(t, n, size, cells) } else { self.alloc_large(t, n, size, cells) }
    }

    #[inline(never)]
    fn alloc_large(&mut self, t: u8, n: usize, bytes: usize, cells: usize) -> Result<Sexp, NeedGc> {
        if self.due && bytes >= LARGE {
            self.large_while_due(t, n, bytes);
        }
        if self.sizes.over_trigger(cells) {
            self.poll_slow(t, n, bytes, cells)?;
        }
        let p = self.heap.try_alloc_large(bytes).ok_or(NeedGc::new(t, n))?;
        // SAFETY: `p` is fresh, with `bytes` bytes.
        let x = unsafe { self.init_header(p, t) };
        self.count(bytes, cells);
        Ok(x)
    }

    /// A `&Cx` request of [`LARGE`] bytes or more while a collection is
    /// due: it should have collected first (docs/g3.md §3 item 3).
    #[cold]
    #[inline(never)]
    fn large_while_due(&mut self, t: u8, n: usize, bytes: usize) {
        self.stats.large_while_due += 1;
        self.stats.largest_while_due = self.stats.largest_while_due.max(bytes as u64);
        if self.stats_mode == StatsMode::Check {
            panic!(
                "robigo: a {bytes}-byte allocation (SEXPTYPE {t}, length {n}) under &Cx while a collection is due: \
                 requests of LARGE bytes or more go through alloc_rooted (docs/g3.md §3 item 3)"
            );
        }
    }

    /// A node (cons cell shape): LISTSXP, LANGSXP, CLOSXP, ENVSXP, ...
    /// Never collects.
    ///
    /// # Safety
    /// `car`, `cdr` and `tag` must be live objects of this heap: they are
    /// reference counted here and traced by every later collection. Only
    /// for EXTPTRSXP, BUILTINSXP and SPECIALSXP may `car` be any word.
    #[inline]
    pub unsafe fn alloc_node(&mut self, t: u8, car: Sexp, cdr: Sexp, tag: Sexp) -> Result<Sexp, NeedGc> {
        let x = self.alloc_object(t, 1, NODE_SIZE, 0)?;
        // read before the field stores, which the compiler cannot tell from
        // a store to `self.nil`: one load, shared with the header's
        let nil = self.nil;
        // SAFETY: `x` has node fields; `car`, `cdr` and `tag` are live (the
        // caller's contract, and nothing collected), and only an object's
        // count is written, never a non-object `car`.
        unsafe {
            x.set_car_raw(car);
            x.set_cdr_raw(cdr);
            x.set_tag_raw(tag);
            x.set_sexp_at(crate::OFF_AUX, nil);
            // GNU R's constructors count the children they store; an
            // external pointer's car is an address and a primitive's an
            // offset, not objects
            // (R_NilValue, the commonest child, is immortal: a pointer
            // comparison spares reading its flags)
            if car != nil && !matches!(t, EXTPTRSXP | BUILTINSXP | SPECIALSXP) {
                car.incref();
            }
            if cdr != nil {
                cdr.incref();
            }
            if tag != nil {
                tag.incref();
            }
        }
        Ok(x)
    }

    /// A vector as GNU R's allocVector: STRSXP filled with
    /// R_BlankString, VECSXP/EXPRSXP with R_NilValue, others not
    /// initialized. Never collects: [`NeedGc`] if it cannot be served
    /// without collecting (a limit, a refused mapping), which the caller
    /// retries under `&mut Cx`.
    ///
    /// # Panics
    /// If `t` is not a vector type (the check folds away for a constant `t`).
    #[inline]
    pub fn alloc_vector(&mut self, t: u8, n: usize) -> Result<Sexp, NeedGc> {
        let esz = crate::elt_size(t);
        if n <= 256 && !matches!(t, STRSXP | VECSXP | EXPRSXP | WEAKREFSXP) {
            if esz == 0 {
                not_a_vector_type(t);
            }
            // small atomic vectors: no overflow check, no fill
            let x = self.alloc_object(t, n, (OFF_DATA + n * esz).div_ceil(16) * 16, vec_cells(t, n))?;
            // SAFETY: at least OFF_DATA bytes were requested, and a fresh
            // vector of a type with elements has its length fields there.
            unsafe {
                x.set_std_len(n as isize);
                x.set_truelen(0);
            }
            return Ok(x);
        }
        self.alloc_vector_slow(t, n)
    }

    fn alloc_vector_slow(&mut self, t: u8, n: usize) -> Result<Sexp, NeedGc> {
        if crate::elt_size(t) == 0 {
            not_a_vector_type(t);
        }
        let (bytes, cells) = vector_size(t, n).ok_or(NeedGc::new(t, n))?;
        let x = self.alloc_sized(t, n, bytes, cells)?;
        // SAFETY: fresh, of `bytes` bytes for `n` elements of `t`.
        unsafe { self.init_vector(x, t, n) };
        Ok(x)
    }

    /// Set a fresh vector's lengths and fill what GNU R fills.
    ///
    /// # Safety
    /// `x` must be fresh, of type `t`, with room for `n` elements.
    unsafe fn init_vector(&self, x: Sexp, t: u8, n: usize) {
        // SAFETY: the caller's contract, and for the filled types the
        // element size is a pointer's, so the `n` writes stay in the payload.
        unsafe {
            x.set_std_len(n as isize);
            x.set_truelen(0);
            let d = x.0.add(OFF_DATA) as *mut Sexp;
            match t {
                STRSXP => {
                    for i in 0..n {
                        *d.add(i) = self.blank_string;
                    }
                }
                VECSXP | EXPRSXP | WEAKREFSXP => {
                    for i in 0..n {
                        *d.add(i) = self.nil;
                    }
                }
                _ => {}
            }
        }
    }

    /// A vector whose payload belongs to someone else: `release(owner)`
    /// is called once the object is dead (after the sweep). Never collects.
    ///
    /// # Safety
    /// `data` must point at `n` elements of type `t` and stay valid until
    /// `release` runs; `release` must accept `owner` after any later sweep.
    pub unsafe fn alloc_extdata(
        &mut self,
        t: u8,
        data: *mut u8,
        n: usize,
        owner: usize,
        release: Option<ReleaseFn>,
    ) -> Result<Sexp, NeedGc> {
        // (the payload is not on this heap: a node, as GNU R counts a
        // custom allocator's vector, and requested as one)
        let x = self.alloc_object(t, 0, EXTDATA_SIZE, 0)?;
        // SAFETY: EXTDATA_SIZE covers every field written, up to +56.
        unsafe {
            x.set_flags(flags::EXTDATA);
            x.set_std_len(n as isize);
            x.set_truelen(0);
            *(x.0.add(32) as *mut *mut u8) = data;
            *(x.0.add(40) as *mut usize) = owner;
            *(x.0.add(48) as *mut Option<ReleaseFn>) = release;
        }
        Ok(x)
    }

    /// The owner word of an EXTDATA vector.
    ///
    /// # Safety
    /// `x` must be a live object made by [`Core::alloc_extdata`] (flag
    /// EXTDATA). On a standard vector +40 is payload.
    pub unsafe fn extdata_owner(x: Sexp) -> usize {
        // SAFETY: the caller's contract: EXTDATA has its owner word at +40.
        unsafe { *(x.0.add(40) as *const usize) }
    }

    // ---- allocation under &mut Cx: it may collect -----------------------

    /// GNU R's allocVector3 for a request made where a collection may run
    /// (docs/g3.md §3 item 3, `cx.alloc_rooted`): collect first if one is
    /// due, gctorture's countdown runs out, or this request makes one due
    /// (`due(size)`), then check the limits, then allocate, collecting once
    /// more if the system refuses the memory. So a large result peaks where
    /// GNU R's does: the garbage goes before it comes.
    ///
    /// # Safety
    /// As for [`Core::collect`]: every edge `roots` shows is null or a live
    /// object of this heap, and every object the caller still needs is
    /// reachable from them. The result is fresh: nothing reaches it yet.
    ///
    /// # Panics
    /// If `t` is not a vector type.
    pub unsafe fn alloc_rooted<R: Trace + ?Sized>(
        &mut self,
        t: u8,
        n: usize,
        roots: &mut R,
    ) -> Result<Sexp, AllocError> {
        if crate::elt_size(t) == 0 {
            not_a_vector_type(t);
        }
        let Some((bytes, cells)) = vector_size(t, n) else {
            return Err(AllocError::CannotAllocate { vecrecs: vecrecs_saturating(t, n) });
        };
        if self.stats_mode != StatsMode::Off {
            self.note_alloc(t, bytes);
        }
        let forced = !self.due && self.force_wait > 0 && self.force_gc();
        let mut collected = false;
        if forced || self.due || self.sizes.due(cells) {
            // SAFETY: the caller's contract.
            collected = unsafe { self.room_for(cells, Safepoint::Rooted, roots)? };
        }
        // Within the limits here: below the trigger, or checked after the
        // collection (or, with collections inhibited, the vector limit, as
        // GNU R's postponed collection checks).
        let mut p = self.alloc_raw(bytes);
        if p.is_none() && !collected && self.gc_inhibit == 0 {
            // R: memory.c:allocVector3, a refused malloc: collect
            // (R_gc_no_finalizers) and try once more
            // SAFETY: the caller's contract.
            unsafe { self.collect_needing(cells, Safepoint::Rooted, roots) };
            p = self.alloc_raw(bytes);
        }
        let Some(p) = p else {
            return Err(AllocError::CannotAllocate { vecrecs: cells });
        };
        // SAFETY: `p` is fresh, of `bytes` (at least 16) bytes for `n`
        // elements of `t`.
        let x = unsafe {
            let x = self.init_header(p, t);
            self.init_vector(x, t, n);
            x
        };
        self.count(bytes, cells);
        Ok(x)
    }

    /// Memory for an object of `size` bytes (at least 16), past the
    /// poll-only path's tests: None if the system refuses a large one.
    #[inline]
    fn alloc_raw(&mut self, size: usize) -> Option<*mut u8> {
        if size <= MAX_MEDIUM { Some(self.heap.alloc(size.max(16))) } else { self.heap.try_alloc_large(size) }
    }

    /// Make room for the request `req` that the poll-only path refused
    /// ([`NeedGc`]), where a collection may run: collect (unless
    /// collections are inhibited), then check GNU R's limits for the
    /// request. `Ok` means the same request, retried at once, passes the
    /// limits; a refused mapping can still fail it, and a second refusal of
    /// the same request after `make_room` is [`AllocError::CannotAllocate`].
    ///
    /// # Safety
    /// As for [`Core::collect`].
    pub unsafe fn make_room<R: Trace + ?Sized>(&mut self, req: NeedGc, roots: &mut R) -> Result<(), AllocError> {
        let cells = request_cells(req)?;
        // SAFETY: the caller's contract.
        unsafe { self.room_for(cells, Safepoint::Retry, roots)? };
        Ok(())
    }

    /// Collect before an allocation of `cells` VECRECs and check the
    /// limits after, or, with collections inhibited, check the vector limit
    /// against what is allocated and leave the collection due. Returns
    /// whether it collected.
    ///
    /// # Safety
    /// As for [`Core::collect`].
    // R: memory.c:allocVector3 (2709-2714), R_gc_internal
    unsafe fn room_for<R: Trace + ?Sized>(
        &mut self,
        cells: usize,
        kind: Safepoint,
        roots: &mut R,
    ) -> Result<bool, AllocError> {
        if self.gc_inhibit > 0 {
            // R_gc_internal's `!R_GCEnabled` branch: mem_err_heap if the
            // vector heap cannot grow, else the collection waits
            if !self.sizes.within_vector_limit(cells) {
                return Err(AllocError::VectorMemory { limit: self.sizes.max_vsize() });
            }
            if !self.due {
                self.set_due();
            }
            return Ok(false);
        }
        // SAFETY: the caller's contract.
        unsafe { self.collect_needing(cells, kind, roots) };
        if self.sizes.nodes_in_use >= self.sizes.n_size {
            return Err(AllocError::ConsMemory { limit: self.sizes.max_nsize() });
        }
        if self.sizes.vheap_free() < cells {
            return Err(AllocError::VectorMemory { limit: self.sizes.max_vsize() });
        }
        Ok(true)
    }

    // ---- CHARSXPs and symbols -------------------------------------------

    fn alloc_char(&mut self, bytes: &[u8], gpbits: u16, h: u32) -> Result<Sexp, NeedGc> {
        let n = bytes.len();
        // (a slice's length leaves room for the header: no overflow)
        let size = (OFF_DATA + n + 1).div_ceil(16) * 16;
        let x = self.alloc_sized(CHARSXP, n, size, vec_cells(CHARSXP, n))?;
        // SAFETY: `x` has room for the bytes and their NUL, and `bytes`, a
        // borrow of memory outside the slot just handed out, cannot overlap it.
        unsafe {
            *(x.0.add(8) as *mut u64) = h as u64;
            x.set_std_len(bytes.len() as isize);
            x.set_truelen(0);
            let d = x.0.add(OFF_DATA);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), d, bytes.len());
            *d.add(bytes.len()) = 0;
            x.set_gp(gpbits);
        }
        Ok(x)
    }

    /// The cached CHARSXP for `bytes` with encoding bits `encbits` (a
    /// combination of `gp::{UTF8, LATIN1, BYTES}`; ASCII is computed here
    /// and replaces the others, as mkCharLenCE does). The caller has
    /// rejected embedded NULs. Never collects.
    pub fn mk_char(&mut self, bytes: &[u8], encbits: u16) -> Result<Sexp, NeedGc> {
        self.mk_char_or(bytes, encbits, || {})
    }

    /// `mk_char`, calling `on_miss` before it creates a CHARSXP: checks
    /// that only a new string can fail (a cached one passed them) cost
    /// nothing on a hit.
    #[inline]
    pub fn mk_char_or(&mut self, bytes: &[u8], encbits: u16, on_miss: impl FnOnce()) -> Result<Sexp, NeedGc> {
        let ascii = bytes.is_ascii();
        let bits = if ascii { gp::ASCII } else { encbits & (gp::UTF8 | gp::LATIN1 | gp::BYTES) } | gp::CACHED;
        let h = hash_bytes(bytes);
        if let Some(c) = self.find_char_hashed(bytes, bits & !gp::CACHED, h) {
            return Ok(c);
        }
        on_miss();
        let c = self.alloc_char(bytes, bits, h)?;
        // SAFETY: `c` is a new CHARSXP of this heap, and this table is its.
        unsafe { self.chars.insert(c) };
        Ok(c)
    }

    /// The cached CHARSXP for `bytes` whose encoding marks (ASCII included)
    /// are exactly `enc`, if there is one.
    pub fn find_char(&self, bytes: &[u8], enc: u16) -> Option<Sexp> {
        self.find_char_hashed(bytes, enc, hash_bytes(bytes))
    }

    #[inline]
    fn find_char_hashed(&self, bytes: &[u8], enc: u16, h: u32) -> Option<Sexp> {
        // SAFETY: the table's entries are live CHARSXPs: the collector removes
        // dead ones before it frees them.
        self.chars.find(h, |c| unsafe { c.gp() & crate::chars::ENC_MASK == enc && c.char_bytes() == bytes })
    }

    fn new_symbol(&mut self, name: Sexp) -> Result<Sexp, NeedGc> {
        let s = self.alloc_object(SYMSXP, 1, NODE_SIZE, 0)?;
        // SAFETY: `s` is fresh and node-sized; `name` is live (its callers
        // make it immortal right after) and `nil` immortal, and nothing
        // collects in between.
        unsafe {
            s.set_car_raw(name);
            s.set_cdr_raw(self.unbound_or(s));
            s.set_tag_raw(self.nil);
            s.set_sexp_at(crate::OFF_AUX, self.nil);
            self.make_immortal(s);
        }
        Ok(s)
    }

    fn unbound_or(&self, s: Sexp) -> Sexp {
        if self.unbound.is_null() { s } else { self.unbound }
    }

    /// The symbol named `name` (GNU R's install). Never collects.
    #[inline]
    pub fn install(&mut self, name: &[u8]) -> Result<Sexp, NeedGc> {
        self.install_named(name, Sexp::NULL)
    }

    /// The symbol named by CHARSXP `c`'s bytes; a new one has `c` itself
    /// as its PRINTNAME, mark and all (names.c:installNoTrChar does this
    /// for ASCII text and for text in the locale's encoding).
    ///
    /// # Safety
    /// `c` must be a live CHARSXP of this heap.
    #[inline]
    pub unsafe fn install_char(&mut self, c: Sexp) -> Result<Sexp, NeedGc> {
        // SAFETY: `c` is live, this function's contract, and stays so:
        // allocation does not collect, and the heap does not move objects.
        let name = unsafe { c.char_bytes() };
        self.install_named(name, c)
    }

    /// One body (one inlined hash lookup) for both: `printname` NULL makes
    /// a native CHARSXP of `name`.
    #[inline(never)]
    fn install_named(&mut self, name: &[u8], printname: Sexp) -> Result<Sexp, NeedGc> {
        if let Some(&s) = self.syms.get(name) {
            return Ok(s);
        }
        let c = if printname.is_null() { self.mk_char(name, 0)? } else { printname };
        let s = self.new_symbol(c)?;
        // SAFETY: `c` is live (nothing collected since it was made or
        // passed), and a CHARSXP has no children. The immortal symbol is
        // not traced, so its name must be immortal too.
        unsafe { self.make_immortal(c) };
        self.syms.insert(name.into(), s);
        Ok(s)
    }

    /// The symbol for a CHARSXP's bytes, or None if not installed.
    pub fn find_symbol(&self, name: &[u8]) -> Option<Sexp> {
        self.syms.get(name).copied()
    }

    /// How many CHARSXPs the cache contains, dead ones not yet pruned included.
    pub fn char_cache_len(&self) -> usize {
        self.chars.len()
    }

    /// How many symbols exist.
    pub fn symbol_count(&self) -> usize {
        self.syms.len()
    }

    /// Every symbol (R_lsInternal-style walks, `ls(baseenv())`).
    pub fn symbols(&self) -> impl Iterator<Item = Sexp> + '_ {
        self.syms.values().copied()
    }

    // ---- roots ---------------------------------------------------------

    // R: memory.c:R_PreserveObject / R_ReleaseObject (the list form):
    // each preserve adds an entry, each release removes the most recent
    // matching one
    ///
    /// # Safety
    /// `x` must be a live object of this heap: every collection traces it
    /// until it is released.
    #[inline]
    pub unsafe fn preserve(&mut self, x: Sexp) {
        self.precious.push(x);
    }

    #[inline]
    pub fn release(&mut self, x: Sexp) {
        if let Some(k) = self.precious.iter().rposition(|&p| p == x) {
            self.precious.swap_remove(k);
        }
    }

    /// Register a weak reference object (a WEAKREFSXP: key, value,
    /// finalizer, next) with the collector.
    ///
    /// # Safety
    /// `w` must be a live, fully initialized WEAKREFSXP of this heap: the
    /// collector keeps it and follows those fields on every collection.
    pub unsafe fn register_weak_ref(&mut self, w: Sexp) {
        // R: memory.c:NewWeakRef prepends to R_weak_refs.
        self.weak_refs.insert(0, w);
    }

    /// Number of references queued before a finalizer batch begins.
    pub fn pending_finalizer_count(&self) -> usize {
        self.finalize_queue.len()
    }

    /// R: memory.c:R_RunExitFinalizers -- mark every onexit reference
    /// ready and move it to the finalizer queue in weak-reference list
    /// order, then let the ordinary runner execute it.
    pub fn queue_exit_finalizers(&mut self) {
        let mut keep = Vec::with_capacity(self.weak_refs.len());
        for w in std::mem::take(&mut self.weak_refs) {
            // SAFETY: every collection marks the objects in `weak_refs`.
            if unsafe { w.gp() } & gp::FINALIZE_ON_EXIT != 0 {
                // SAFETY: as above.
                unsafe { w.set_gp(w.gp() | gp::READY_TO_FINALIZE) };
                self.finalize_queue.push(w);
            } else {
                keep.push(w);
            }
        }
        self.weak_refs = keep;
    }

    /// The next weak reference whose key died, ready for its finalizer.
    /// It stays rooted until popped; the caller protects it while it runs.
    pub fn pop_finalizable(&mut self) -> Option<Sexp> {
        if self.finalize_queue.is_empty() { None } else { Some(self.finalize_queue.remove(0)) }
    }

    /// Release callbacks of external buffers the last
    /// collections found dead. They never run inside a collection; the
    /// API runs them at safe points.
    pub fn take_releases(&mut self) -> Vec<(ReleaseFn, usize)> {
        std::mem::take(&mut self.releases)
    }

    // ---- C segments ----------------------------------------------------
    //
    // The conservative scan reads the frames of package C code and nothing
    // else (docs/g3.md §3: no Rust frame is ever scanned). A C segment runs
    // from a native entry down to the export the C code calls:
    //
    // - the native entry opens it: `robigo_heap_try_call65` sets its top
    //   just above the routine's stack arguments while the routine runs;
    // - an export closes it first thing ([`Core::close_scan_segment`]): its
    //   low end is the export's machine entry, below which only Rust
    //   frames lie; the export reopens it as it returns.
    //
    // A collection reads the closed segments only, and refuses to run with
    // one open: an export that collects has closed its caller's segment, and
    // nothing else can collect while C code runs.

    /// Open the C segment at `top` (native code below it is scanned while it
    /// runs), or close it with 0. Returns the previous top. Native entries
    /// set it: `robigo_heap_try_call65` exactly, at the routine's stack
    /// arguments.
    pub fn set_scan_top(&mut self, top: usize) -> usize {
        std::mem::replace(&mut self.scan_top, top)
    }

    /// The open C segment's top (0: none; see [`Core::set_scan_top`]).
    pub fn scan_top(&self) -> usize {
        self.scan_top
    }

    /// Where the open segment's top is stored, for the C trampoline that
    /// opens one at a native entry (`unwind::robigo_heap_try_call65`).
    pub fn scan_top_slot(&mut self) -> *mut usize {
        &mut self.scan_top
    }

    /// C code called an export: close the open C segment, from `lo` up to
    /// its top, recorded in `seg`. From here until
    /// [`Core::reopen_scan_segment`] nothing below `lo` is scanned: the
    /// export and the code it runs keep objects only through precise roots,
    /// and their stack words, stale ones of earlier frames included, decide
    /// nothing.
    ///
    /// `lo` is exact, and the segment holds no Rust word, when the export's
    /// machine entry pushed the callee-saved registers C left in place just
    /// below its return address, before any Rust code of the export ran,
    /// and passes the lowest of their addresses: the mirror of the native
    /// entry's stub (`tests/heap.rs`'s `routine_collect` is such an entry
    /// on x86-64). An export without one spills the registers into `seg`
    /// ([`ScanSegment::spill_registers`]), whose `regs` are scanned with
    /// the segment, and passes `seg`'s address: then the words of its own
    /// frame above the node are in the segment too, and stale ones among
    /// them keep what they point at alive (and, under today's order, a weak
    /// reference's key: H2 step 6 ends that).
    ///
    /// # Safety
    /// Every word of the C frames, and every callee-saved register value C
    /// left in place, must be in `[lo, top)` or in `seg`'s spilled
    /// registers, and `lo` above the frames of the collector that may run
    /// before the reopen. `seg` must stay valid, and in place, until the
    /// matching reopen, which comes after those of segments closed later.
    #[inline(always)]
    pub unsafe fn close_scan_segment(&mut self, seg: *mut ScanSegment, lo: usize) {
        // SAFETY: the caller's contract.
        unsafe {
            (*seg).lo = lo;
            (*seg).hi = self.scan_top;
            (*seg).prev = self.scan_chain;
        }
        self.scan_chain = seg;
        self.scan_top = 0;
    }

    /// Undo [`Core::close_scan_segment`] of `seg`, as the export returns
    /// to the C code.
    ///
    /// # Safety
    /// `seg` must be the innermost closed segment.
    #[inline(always)]
    pub unsafe fn reopen_scan_segment(&mut self, seg: *const ScanSegment) {
        // SAFETY: the caller's contract.
        unsafe {
            self.scan_chain = (*seg).prev;
            self.scan_top = (*seg).hi;
        }
    }

    /// The thread's stack bounds the core scans: (lowest, highest).
    pub fn stack_bounds(&self) -> (usize, usize) {
        (self.stack_lo, self.stack_top)
    }

    /// gctorture's countdown, once per allocation while it is on.
    // R: memory.c:FORCE_GC
    #[cold]
    #[inline(never)]
    fn force_gc(&mut self) -> bool {
        self.force_wait -= 1;
        if self.force_wait > 0 {
            return false;
        }
        self.force_wait = self.force_gap;
        true
    }

    /// Whether gctorture is on (`gctorture()`'s result: a forced
    /// collection is pending).
    pub fn torture(&self) -> bool {
        self.force_wait > 0
    }

    /// A collection due at every allocation (`gctorture(on)`): served at
    /// the next safepoint.
    pub fn set_torture(&mut self, on: bool) {
        self.gc_torture(on as i32, 0);
    }

    /// The torture counters: (allocations until the next forced
    /// collection, allocations between them); (0, 0) when off.
    pub fn torture_counters(&self) -> (i32, i32) {
        (self.force_wait, self.force_gap)
    }

    /// A collection due every `gap` allocations, the first after `wait` of
    /// them (`gap` 0 turns torture off; negative or NA values change
    /// nothing). Allocation never collects: each comes due, and the next
    /// safepoint serves it.
    // R: memory.c:R_gc_torture (without PROTECTCHECK: no inhibit_release)
    pub fn gc_torture(&mut self, gap: i32, wait: i32) {
        if gap >= 0 {
            self.force_wait = gap;
            self.force_gap = gap;
        }
        if gap > 0 && wait > 0 {
            self.force_wait = wait;
        }
        self.retrigger();
    }

    /// Weak references still waiting for their key to die (for
    /// finalizers that run on exit).
    pub fn weak_refs(&self) -> &[Sexp] {
        &self.weak_refs
    }

    /// Every field of every weak reference the collector still tracks (key,
    /// value, finalizer), for both entries awaiting classification and ones
    /// queued for RunFinalizers. GNU R keeps key, value and finalizer alive
    /// for the finalization round (memory.c RunGenCollect), so the runtime
    /// must not sweep what only they reach yet.
    pub fn weak_ref_fields(&self) -> Vec<Sexp> {
        self.weak_refs
            .iter()
            .chain(self.finalize_queue.iter())
            // SAFETY: both lists contain initialized WEAKREFSXPs that every
            // collection marks.
            .flat_map(|&w| (0..3).map(move |i| unsafe { Self::weak_field(w, i) }))
            .collect()
    }

    /// Bytes reserved from the OS.
    pub fn heap_reserved(&self) -> usize {
        self.heap.reserved
    }

    /// Whether `x` points into an object of this heap (start or interior).
    pub fn resolve(&self, w: usize) -> Option<Sexp> {
        self.heap.resolve(w).map(Sexp)
    }

    /// The lowest and one past the highest address of the heap's chunks.
    pub fn heap_bounds(&self) -> (usize, usize) {
        (self.heap.min_addr, self.heap.max_addr)
    }

    /// Whether `w` lies in memory this heap reserved (an object, free
    /// space or a block header): no claim that an object starts there.
    pub fn in_heap(&self, w: usize) -> bool {
        self.heap.chunk_of(w).is_some()
    }

    /// This heap's identity, which every object it allocated carries
    /// ([`Sexp::heap_id`]): the address of its remembered set, which no other
    /// live heap shares.
    #[inline(always)]
    pub fn id(&self) -> crate::HeapId {
        crate::HeapId(&*self.remembered as *const Remembered as usize)
    }

    // ---- collection ----------------------------------------------------

    /// # Safety
    /// `p` must be null or the start of an object of this heap that is not
    /// free: it, and later everything it reaches, is read.
    #[inline]
    unsafe fn mark_obj(&mut self, p: Sexp) {
        // SAFETY: the caller's contract. A live object's block is its address
        // rounded down to 64 KiB (a large object's is its chunk's base), and
        // either kind starts with a word that tells them apart. In a block `g`
        // is a granule of the data area, so the bitmap word is in `marks`.
        unsafe {
            if p.is_null() {
                return;
            }
            if p.flags() & flags::IMMORTAL != 0 {
                return;
            }
            // marks live beside the objects (the block's mark bitmap, a
            // large chunk's header word): marking never writes an object
            let b = (p.0 as usize & !(BLOCK - 1)) as *mut Block;
            if (*b).magic == 0x524f4249474f4241 {
                let g = (p.0 as usize - Block::data_ptr(b) as usize) / 16;
                let w = (*b).marks.as_mut_ptr().add(g >> 6);
                let bit = 1u64 << (g & 63);
                if *w & bit != 0 {
                    return;
                }
                *w |= bit;
                (*b).marked = (*b).marked.wrapping_add(1);
            } else {
                let m = (b as *mut u64).add(1);
                if *m == self.epoch {
                    return;
                }
                *m = self.epoch;
            }
            self.mark_stack.push(p);
        }
    }

    /// # Safety
    /// As for [`Core::mark_obj`], except that `p` must not be null.
    #[inline]
    unsafe fn is_marked(&self, p: Sexp) -> bool {
        // SAFETY: as in `mark_obj`, for reads.
        unsafe {
            if p.flags() & flags::IMMORTAL != 0 {
                return true;
            }
            let b = (p.0 as usize & !(BLOCK - 1)) as *mut Block;
            if (*b).magic == 0x524f4249474f4241 {
                let g = (p.0 as usize - Block::data_ptr(b) as usize) / 16;
                (*b).marks[g >> 6] & (1u64 << (g & 63)) != 0
            } else {
                *(b as *const u64).add(1) == self.epoch
            }
        }
    }

    /// Mark what the objects on the mark stack reach.
    ///
    /// # Safety
    /// Only during a collection, on a mark stack of live objects whose
    /// fields contain live objects (the rules of the raw setters).
    unsafe fn trace(&mut self) {
        while let Some(p) = self.mark_stack.pop() {
            // SAFETY: the caller's contract: `p` is live, its fields are null
            // or live, and the marker reaches objects only through the edges
            // it is shown.
            unsafe {
                if p.typ() == CHARSXP {
                    // every cached CHARSXP is in the table (mk_char)
                    self.marked_chars += (p.gp() & gp::CACHED != 0) as usize;
                    continue;
                }
                p.trace_fields(&mut Mark(self));
            }
        }
    }

    /// Field `i` of a weak reference: key, value, finalizer.
    ///
    /// # Safety
    /// `w` must be a live, standard WEAKREFSXP longer than `i`.
    unsafe fn weak_field(w: Sexp, i: usize) -> Sexp {
        // SAFETY: element `i` of a standard vector's pointer payload.
        unsafe { *(w.std_data() as *const Sexp).add(i) }
    }

    /// GNU R's weak-reference protocol (memory.c RunGenCollect): values
    /// and finalizers of references with live keys are live; references
    /// whose key died and that have a finalizer are queued, keeping key,
    /// value and finalizer alive for the finalizer; the others are
    /// cleared.
    ///
    /// # Safety
    /// Only during a collection, after the roots are marked.
    unsafe fn process_weak_refs(&mut self) {
        // SAFETY: `register_weak_ref`'s contract makes every entry an
        // initialized WEAKREFSXP. `collect` marked them, and nothing has been
        // swept since, so their fields still point at objects.
        unsafe {
            loop {
                let mut changed = false;
                for i in 0..self.weak_refs.len() {
                    let w = self.weak_refs[i];
                    let key = Self::weak_field(w, 0);
                    if self.is_marked(key) {
                        for k in 1..3 {
                            let v = Self::weak_field(w, k);
                            if !self.is_marked(v) {
                                self.mark_obj(v);
                                changed = true;
                            }
                        }
                    }
                }
                self.trace();
                if !changed {
                    break;
                }
            }
            // memory.c CheckFinalizers: every reference whose key died is
            // ready to finalize (with or without a finalizer); then every
            // reference keeps its key, value and finalizer alive for this
            // round; RunFinalizers clears them
            let refs = std::mem::take(&mut self.weak_refs);
            let mut keep = Vec::with_capacity(refs.len());
            for w in refs {
                let key = Self::weak_field(w, 0);
                if w.gp() & gp::READY_TO_FINALIZE != 0 || !self.is_marked(key) {
                    w.set_gp(w.gp() | gp::READY_TO_FINALIZE);
                    self.finalize_queue.push(w);
                } else {
                    keep.push(w);
                }
            }
            // R: memory.c:RunGenCollect determines every ready key before
            // keeping keys/finalizers alive for this collection. Marking
            // during classification would defer another finalizer of the
            // same key (or a mutually reachable key) by one collection.
            for &w in &keep {
                for k in 0..3 {
                    self.mark_obj(Self::weak_field(w, k));
                }
            }
            for i in 0..self.finalize_queue.len() {
                let w = self.finalize_queue[i];
                for k in 0..3 {
                    self.mark_obj(Self::weak_field(w, k));
                }
            }
            self.trace();
            self.weak_refs = keep;
        }
    }

    /// Mark what the closed C segments point at (and what that reaches).
    /// The collector's own frames, and every Rust frame, are not read.
    ///
    /// # Safety
    /// `ctx` must be the core `collect_needing` passed to
    /// `robigo_heap_spill`, the only way to reach it during this call, and
    /// the thread must be the core's.
    unsafe extern "C" fn scan_segments(ctx: *mut c_void) {
        // SAFETY: `ctx` is as the caller promises. Each segment is mapped
        // stack of this thread (collect checked the thread, and a segment
        // lies in frames that are suspended above this one) and word-aligned,
        // and `resolve` only returns allocated objects. This is the one block
        // in the crate whose reason lies outside Rust's rules: the loop reads
        // other frames' memory, which can be uninitialized, or borrowed
        // exclusively by a frame that is suspended here. A conservative scan
        // needs those reads. It works because they are ordinary machine
        // loads of integers compared to addresses, in a function only called
        // through C, so nothing can be optimized from them.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let mut seg = c.scan_chain;
            while !seg.is_null() {
                let s = &*seg;
                c.scan_words(s.lo.max(sp), s.hi.min(c.stack_top));
                let regs = s.regs.as_ptr() as usize;
                c.scan_words(regs, regs + std::mem::size_of_val(&s.regs));
                seg = s.prev;
            }
            c.trace();
        }
    }

    /// Mark what the words in `[from, to)` point at: a range of this
    /// thread's stack, or a segment node's spilled registers.
    ///
    /// # Safety
    /// As [`Core::scan_segments`]'s loop: `[from, to)` is readable memory of
    /// this thread, read while marking.
    unsafe fn scan_words(&mut self, from: usize, to: usize) {
        // SAFETY: the caller's contract; see `scan_segments`.
        unsafe {
            let (min, max) = (self.heap.min_addr, self.heap.max_addr);
            let mut w = (from + 7) & !7;
            while w < to {
                let v = *(w as *const usize);
                // (`max` itself is no object's address, but it is one past
                // the end of the highest chunk's last object)
                if v >= min && v <= max {
                    if v < max {
                        if let Some(p) = self.heap.resolve(v) {
                            self.mark_obj(Sexp(p));
                        }
                    }
                    // a word on a granule boundary may be one past the end
                    // of the previous object's data (a loop bound, p + n)
                    if v & 15 == 0 && v > min {
                        if let Some(p) = self.heap.resolve(v - 1) {
                            if has_inline_data(Sexp(p)) {
                                self.mark_obj(Sexp(p));
                            }
                        }
                    }
                }
                w += 8;
            }
        }
    }

    /// A full collection (`gc()`, `R_gc()`), whether or not one is due and
    /// whether or not collections are inhibited. Returns the pause in
    /// nanoseconds. Release callbacks of dead EXTDATA objects are queued
    /// ([`Core::take_releases`]); queued finalizers do not run here (see
    /// [`Core::pop_finalizable`]).
    ///
    /// # Safety
    /// Every edge `roots` shows is null or a live object of this heap, and
    /// every object the caller still needs is reachable from `roots`, the
    /// heap's own roots (the precious set, weak references, the store
    /// barrier's log) or a closed C segment: nothing else keeps one alive.
    ///
    /// # Panics
    /// If a C segment is open (C code runs, and the export that collects
    /// did not close its segment), if a collection is already running, or
    /// if called from another thread.
    // R: memory.c:R_gc -> R_gc_internal(0)
    pub unsafe fn collect<R: Trace + ?Sized>(&mut self, roots: &mut R) -> u64 {
        // SAFETY: the caller's contract.
        unsafe { self.collect_needing(0, Safepoint::Explicit, roots) }
    }

    /// A safepoint of kind `kind`: collect if one is due ([`GC_DUE`]) and
    /// collections are not inhibited. Returns whether it collected.
    ///
    /// # Safety
    /// As for [`Core::collect`].
    #[inline]
    pub unsafe fn safepoint<R: Trace + ?Sized>(&mut self, kind: Safepoint, roots: &mut R) -> bool {
        if !self.due || self.gc_inhibit > 0 {
            return false;
        }
        // SAFETY: the caller's contract.
        unsafe { self.collect_needing(0, kind, roots) };
        true
    }

    /// A full collection before allocating `size_needed` VECRECs; the heap
    /// sizes are adjusted for it afterwards. Generic in the roots only:
    /// the rest is [`Core::begin_collection`] and
    /// [`Core::finish_collection`].
    ///
    /// # Safety
    /// As for [`Core::collect`].
    // R: memory.c:R_gc_internal (RunGenCollect, then AdjustHeapSize after a
    // collection of every generation: here each one is)
    #[inline(never)]
    unsafe fn collect_needing<R: Trace + ?Sized>(&mut self, size_needed: usize, kind: Safepoint, roots: &mut R) -> u64 {
        let t0 = self.begin_collection();
        // SAFETY: nothing was freed since the last sweep, so the heap's own
        // lists contain objects whose fields are objects (the rooting
        // contracts of `preserve`, `register_weak_ref` and the store
        // barrier); the caller vouches for `roots`.
        unsafe {
            self.mark_heap_roots();
            roots.trace(&mut Mark(self));
            self.finish_collection(size_needed, kind, t0)
        }
    }

    /// Start a collection: check the thread and the segments, start a new
    /// epoch. Returns the start time and what was allocated since the last
    /// collection.
    fn begin_collection(&mut self) -> (Instant, usize) {
        assert!(!self.collecting, "robigo: a collection inside a collection");
        assert!(
            self.scan_top == 0,
            "robigo: a collection while C code runs with its segment open: the export that collects must close it first (Core::close_scan_segment)"
        );
        let sp = stack_pointer();
        if sp < self.stack_lo || sp > self.stack_top {
            eprintln!("robigo: the R heap was used from a thread other than the R thread");
            std::process::abort();
        }
        self.collecting = true;
        let allocated = self.alloc_since_gc;
        self.stats.bytes_allocated += allocated as u64;
        self.epoch += 1;
        self.marked_chars = 0;
        (Instant::now(), allocated)
    }

    /// Mark the heap's own precise roots: the precious set, the weak
    /// references and those queued for their finalizers, the store
    /// barrier's log of immortal objects.
    ///
    /// # Safety
    /// During a collection, before anything is swept.
    unsafe fn mark_heap_roots(&mut self) {
        // SAFETY: the caller's contract; see `collect_needing`.
        unsafe {
            for i in 0..self.precious.len() {
                let p = self.precious[i];
                self.mark_obj(p);
            }
            for i in 0..self.finalize_queue.len() {
                let w = self.finalize_queue[i];
                self.mark_obj(w);
                for k in 0..3 {
                    self.mark_obj(Self::weak_field(w, k));
                }
            }
            for i in 0..self.weak_refs.len() {
                let w = self.weak_refs[i];
                self.mark_obj(w);
            }
            // immortal objects are never marked, but their fields can
            // change (a symbol's value, attributes set on a constant): the
            // store barrier logged them (nothing stores during a collection,
            // so the log is not borrowed elsewhere)
            let log = &*self.remembered.0.get();
            self.mark_stack.extend_from_slice(log);
        }
    }

    /// The rest of a collection once the precise roots are marked: what
    /// they reach, weak references and C segments in the order below, the
    /// CHARSXP cache, the sweep; then the heap sizes, the due bit and the
    /// statistics.
    ///
    /// # Safety
    /// During a collection, after `begin_collection` and the precise roots.
    unsafe fn finish_collection(
        &mut self,
        size_needed: usize,
        kind: Safepoint,
        (t0, allocated): (Instant, usize),
    ) -> u64 {
        // SAFETY: the caller's contract; `spill` gets a callback and a
        // context of matching types, on the core's thread.
        unsafe {
            self.trace();
            // docs/g3.md §3, "What a collection marks": precise roots (above),
            // weak references and finalizers decided on them alone, then the
            // C segments as pins. That order is experiment A's
            // (`ROBIGO_GC_PIN_ONLY=1`); until M0's adoption rule has passed
            // (§15.1, H2 step 6) the C words stay strong roots, scanned before
            // the weak pass. The verdict deletes `pin_only` and the `else`.
            if self.pin_only {
                self.process_weak_refs();
                crate::unwind::robigo_heap_spill(Self::scan_segments, self as *mut Core as *mut c_void);
            } else {
                crate::unwind::robigo_heap_spill(Self::scan_segments, self as *mut Core as *mut c_void);
                self.process_weak_refs();
            }
            self.prune_chars();
            self.sweep();
        }
        self.sizes.adjust(size_needed);
        // this collection serves the one due: the overshoot ends here
        if self.due {
            let over = allocated.saturating_sub(self.due_at) as u64;
            let st = &mut self.stats;
            st.total_overshoot += over;
            if over > st.max_overshoot {
                st.max_overshoot = over;
                st.max_overshoot_live = st.live_bytes;
            }
            if over > overshoot_bound(st.live_bytes) {
                st.overshoot_over_bound += 1;
            }
            self.due = false;
            self.poll_word().fetch_and(!GC_DUE, Ordering::Relaxed);
        }
        self.retrigger();
        self.collecting = false;
        let dt = t0.elapsed().as_nanos() as u64;
        self.stats.collections += 1;
        self.stats.collections_by_kind[kind as usize] += 1;
        self.stats.collect_total_ns += dt;
        self.stats.collect_max_pause_ns = self.stats.collect_max_pause_ns.max(dt);
        dt
    }

    fn prune_chars(&mut self) {
        if self.marked_chars + self.immortal_chars == self.chars.len() {
            // SAFETY: nothing has been swept, so entries are still objects.
            debug_assert!(self.chars.all(|c| unsafe { self.is_marked(c) }));
            return;
        }
        let mut chars = std::mem::take(&mut self.chars);
        // SAFETY: nothing has been swept, so entries are still objects.
        chars.prune(|c| unsafe { self.is_marked(c) });
        self.chars = chars;
    }

    /// Queue the release callback of a dead object.
    ///
    /// # Safety
    /// `p` must be an object not yet overwritten by the sweep.
    #[inline]
    unsafe fn on_death(&mut self, p: *mut u8) {
        // SAFETY: the flags are in the header. They mean the object was made
        // by `alloc_extdata`, which fills +40 and +48, and +48
        // is an `Option<ReleaseFn>`: API code that disarms it writes null.
        unsafe {
            // R: memory.c:ReleaseLargeFreeVectors (R_LargeVallocSize -=
            // getVecSizeInVEC(s)); small vectors are counted by class there,
            // which is the same number
            self.sizes.valloc = self.sizes.valloc.saturating_sub(crate::heapsize::object_cells(Sexp(p)));
            let f = *p.add(1);
            if f & flags::EXTDATA != 0 {
                // EXTDATA objects keep the owner at +40 and the release
                // function at +48
                let (owner, rel) = (*(p.add(40) as *const usize), *(p.add(48) as *const Option<ReleaseFn>));
                if let Some(rel) = rel {
                    self.releases.push((rel, owner));
                }
            }
        }
    }

    /// Mark-region sweep (mm-gc's, with skip-clean blocks): dead objects
    /// go back to their block's free list, empty blocks to the pool,
    /// dead chunks to the OS.
    fn sweep(&mut self) {
        let mut live_bytes = 0u64;
        let mut live_objects = 0u64;
        let mut freed = 0u64;
        let mut to_list: Vec<(usize, *mut Block)> = Vec::new();
        let mut to_pool: Vec<*mut Block> = Vec::new();
        let mut dead_chunks: Vec<usize> = Vec::new();
        let mut dead_small_chunks: Vec<(usize, usize, usize)> = Vec::new();
        let chunks = self.heap.chunks.clone();
        let epoch = self.epoch;
        for (idx, c) in chunks.iter().enumerate() {
            if c.large {
                let obj = (c.base + DATA_OFF) as *mut u8;
                // SAFETY: a large chunk contains one object at DATA_OFF, and it
                // has a header once allocated: the chunk is made for it.
                let f = unsafe { *obj.add(1) };
                // SAFETY: +8 of a chunk at least a block long.
                if f & flags::IMMORTAL != 0 || unsafe { *(c.base as *const u64).add(1) } == epoch {
                    live_bytes += (c.end - c.base) as u64;
                    live_objects += 1;
                } else {
                    // SAFETY: as above, and this object is not read again.
                    unsafe { self.on_death(obj) };
                    dead_chunks.push(idx);
                }
                continue;
            }
            let mut chunk_dead = true;
            let mut chunk_touched = false;
            for bi in 0..c.claimed as usize {
                let b = (c.base + bi * BLOCK) as *mut Block;
                // SAFETY: `b` is a claimed block of a live chunk, so its
                // header is initialized. `g` comes from a start bit, so `p`
                // is an object's first byte: its slot, of the class's size,
                // is inside the data area and writable, and has the header
                // and link words this writes.
                unsafe {
                    if (*b).class == CLASS_NONE {
                        if (*b).owner == OWNER_EMPTY {
                            chunk_touched = true;
                        }
                        continue;
                    }
                    if (*b).live_objects > 0 && (*b).marked + (*b).immortals == (*b).live_objects {
                        // nothing died here: skip the bitmap walk
                        live_bytes += (*b).live_objects as u64 * SIZES[(*b).class as usize] as u64;
                        live_objects += (*b).live_objects as u64;
                        (*b).marked = 0;
                        (*b).marks = [0; START_WORDS];
                        chunk_dead = false;
                        // the allocation cursors are reset below: a block
                        // with room left must be findable again
                        if (*b).owner == OWNER_NONE && block_has_room(b) {
                            to_list.push(((*b).class as usize, b));
                        }
                        continue;
                    }
                    let old_starts = (*b).starts;
                    let marks = (*b).marks;
                    (*b).marks = [0; START_WORDS];
                    (*b).starts = [0; START_WORDS];
                    // the walk adds to the free list rather than rebuilding
                    // it: slots already there have no start bit, so it could
                    // not find them again, and for the same reason it cannot
                    // add one twice
                    let mut live: u32 = 0;
                    let csize = SIZES[(*b).class as usize] as u64;
                    for (w, &word) in old_starts.iter().enumerate() {
                        let mut s = word;
                        while s != 0 {
                            let g = w * 64 + s.trailing_zeros() as usize;
                            let p = Block::data_ptr(b).add(g * 16);
                            let f = *p.add(1);
                            if marks[w] & (1u64 << (g & 63)) != 0 || f & flags::IMMORTAL != 0 {
                                live += 1;
                                live_bytes += csize;
                                (*b).start_set(g);
                            } else {
                                self.on_death(p);
                                if self.force_gap > 0 {
                                    // poison: a use after free reads garbage
                                    // types and pointers, and fails loudly
                                    std::ptr::write_bytes(p.add(16), 0xDB, csize as usize - 16);
                                    *p = 0xDB;
                                }
                                *((p as *mut *mut u8).add(1)) = (*b).freelist;
                                (*b).freelist = p;
                                // a free slot reads as an unmarked, flagless object
                                *p.add(1) = 0;
                                freed += 1;
                            }
                            s &= s - 1;
                        }
                    }
                    (*b).marked = 0;
                    (*b).live_objects = live;
                    live_objects += live as u64;
                    if live == 0 {
                        if (*b).owner == OWNER_EMPTY {
                            chunk_touched = true;
                            continue;
                        }
                        if (*b).bump_next == 0 && (*b).sticky_listed == 0 && (*b).owner == OWNER_NONE {
                            continue;
                        }
                        (*b).owner = OWNER_EMPTY;
                        (*b).class = CLASS_NONE;
                        (*b).bump_next = 0;
                        (*b).sticky_listed = 0;
                        (*b).starts = [0; START_WORDS];
                        (*b).freelist = std::ptr::null_mut();
                        to_pool.push(b);
                        chunk_touched = true;
                        continue;
                    }
                    chunk_dead = false;
                    chunk_touched = true;
                    if (*b).owner == OWNER_NONE && block_has_room(b) {
                        to_list.push(((*b).class as usize, b));
                    }
                }
            }
            if chunk_dead && chunk_touched {
                dead_small_chunks.push((idx, c.base, c.end));
            }
        }
        for (ci, b) in to_list {
            // SAFETY: `to_list` contains blocks found above in chunks that are
            // not released until later in this function.
            unsafe {
                (*b).owner = crate::heap::OWNER_LIST;
                (*b).sticky_listed = 1;
            }
            self.heap.classes[ci].blocks.push(b);
        }
        for b in to_pool {
            self.heap.empty_pool.push(b);
        }
        for &(_, base, end) in &dead_small_chunks {
            self.heap.empty_pool.retain(|&b| !(b as usize >= base && (b as usize) < end));
            for cs in &mut self.heap.classes {
                cs.blocks.retain(|&b| !(b as usize >= base && (b as usize) < end));
            }
        }
        let mut release: Vec<usize> = dead_chunks;
        release.extend(dead_small_chunks.iter().map(|&(idx, _, _)| idx));
        release.sort_unstable();
        release.dedup();
        for &idx in release.iter().rev() {
            let c = self.heap.chunks[idx];
            let total = c.end - c.base;
            // SAFETY: `try_new_chunk` allocated exactly this layout, and the
            // loops above dropped every list entry that points into it.
            unsafe {
                std::alloc::dealloc(c.base as *mut u8, std::alloc::Layout::from_size_align(total, BLOCK).unwrap());
            }
            self.heap.reserved -= total;
            self.heap.chunks.remove(idx);
        }
        if let (Some(first), Some(last)) = (self.heap.chunks.first(), self.heap.chunks.last()) {
            self.heap.min_addr = first.base;
            self.heap.max_addr = last.end;
        } else {
            self.heap.min_addr = usize::MAX;
            self.heap.max_addr = 0;
        }
        for cs in &mut self.heap.classes {
            cs.cur = std::ptr::null_mut();
        }
        self.stats.frees += freed;
        self.stats.live_bytes = live_bytes;
        self.stats.live_objects = live_objects;
        self.alloc_since_gc = 0;
        // R: memory.c:RunGenCollect (R_NodesInUse = R_NSize - R_Collected)
        self.sizes.nodes_in_use = live_objects as usize;
    }
}

/// The marking collector's visitor: each edge it is shown is marked, and
/// later traced from.
struct Mark<'a>(&'a mut Core);

impl Visitor for Mark<'_> {
    #[inline(always)]
    fn edge(&mut self, edge: &mut Sexp) {
        // SAFETY: a `Mark` exists only inside a collection (this module makes
        // it), where it is shown the edges of the roots whose objects the
        // collection's caller vouches for, and those of objects marked from
        // them: each is null or live.
        unsafe { self.0.mark_obj(*edge) }
    }
}

/// SEXPTYPE names for the statistics (Rinternals.h).
const TYPE_NAMES: [&str; 26] = [
    "NILSXP",
    "SYMSXP",
    "LISTSXP",
    "CLOSXP",
    "ENVSXP",
    "PROMSXP",
    "LANGSXP",
    "SPECIALSXP",
    "BUILTINSXP",
    "CHARSXP",
    "LGLSXP",
    "11",
    "12",
    "INTSXP",
    "REALSXP",
    "CPLXSXP",
    "STRSXP",
    "DOTSXP",
    "ANYSXP",
    "VECSXP",
    "EXPRSXP",
    "BCODESXP",
    "EXTPTRSXP",
    "WEAKREFSXP",
    "RAWSXP",
    "OBJSXP",
];

impl Core {
    /// What `ROBIGO_GC_STATS` asks for (read when the core is made).
    pub fn stats_mode(&self) -> StatsMode {
        self.stats_mode
    }

    /// Change it: the allocation counts by type start from here.
    pub fn set_stats_mode(&mut self, mode: StatsMode) {
        self.stats_mode = mode;
        self.retrigger();
    }

    /// `ROBIGO_GC_STATS`'s report, printed to stderr when the core is
    /// dropped: collections by safepoint kind, E4's overshoot, large `&Cx`
    /// requests made while a collection was due, allocations by SEXPTYPE.
    pub fn stats_report(&self) -> String {
        use std::fmt::Write;
        let st = &self.stats;
        let mut r = String::new();
        let kinds: Vec<String> =
            Safepoint::ALL.iter().map(|k| format!("{} {}", k.name(), st.collections_by_kind[*k as usize])).collect();
        let _ = writeln!(r, "robigo gc: collections {} ({})", st.collections, kinds.join(", "));
        let _ = writeln!(
            r,
            "robigo gc: overshoot max {} bytes (live {} bytes after it), total {} bytes, over max(32 MiB, 10% live) {}",
            st.max_overshoot, st.max_overshoot_live, st.total_overshoot, st.overshoot_over_bound
        );
        let _ = writeln!(
            r,
            "robigo gc: large &Cx requests while due {} (largest {} bytes)",
            st.large_while_due, st.largest_while_due
        );
        let _ = writeln!(
            r,
            "robigo gc: allocated {} objects, {} bytes",
            st.allocs,
            st.by_type.iter().map(|e| e.1).sum::<u64>()
        );
        for (t, &(objects, bytes)) in st.by_type.iter().enumerate() {
            if objects > 0 {
                let name = TYPE_NAMES.get(t).copied().unwrap_or("?");
                let _ = writeln!(r, "robigo gc:   {name} {objects} objects, {bytes} bytes");
            }
        }
        r
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if self.stats_mode != StatsMode::Off {
            eprint!("{}", self.stats_report());
        }
    }
}

/// [`Core::heap_sizes`]: the collection trigger's state in GNU R's units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapReport {
    /// `R_NSize`: nodes the heap may have before a collection.
    pub n_size: usize,
    /// `R_VSize`: the vector heap in VECRECs.
    pub v_size: usize,
    /// Nodes in use.
    pub nodes_in_use: usize,
    /// VECRECs in use (`R_VSize` counts against these).
    pub vecrecs_in_use: usize,
}
