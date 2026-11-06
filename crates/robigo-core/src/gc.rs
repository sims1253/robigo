//! The collector and the allocation entry points: a non-moving
//! mark-region heap (blocks, size classes: `heap.rs`) with precise roots
//! (protect stack, precious set, registered root callbacks, weak
//! references) and a conservative scan of the machine stack. Ported from
//! the bake-off's `mm-gc` (`experiments/memory`), which measured this
//! design against reference counting (`docs/ledger/memory.md`).

use crate::heap::{BLOCK, Block, CLASS_NONE, DATA_OFF, Heap, OWNER_EMPTY, OWNER_NONE, SIZES, START_WORDS};
use crate::heapsize::HeapSizes;
use crate::types::*;
use crate::{EXTDATA_SIZE, NODE_SIZE, OFF_DATA, Sexp, flags, gp, hash_bytes};
use std::collections::HashMap;
use std::ffi::c_void;
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

/// Releases an external payload or host handle when its object dies.
pub type ReleaseFn = unsafe extern "C" fn(usize);

// Stored at +48 of EXTDATA and HOST objects, which C can see: one word, and
// null for None (a guarantee of `Option<fn>`, so capi's disarming write of 0
// is `None`).
const _: () = assert!(std::mem::size_of::<Option<ReleaseFn>>() == std::mem::size_of::<usize>());

/// Counters the ledger records.
#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub allocs: u64,
    pub bytes_allocated: u64,
    pub collections: u64,
    pub frees: u64,
    pub live_bytes: u64,
    pub live_objects: u64,
    pub collect_total_ns: u64,
    pub collect_max_pause_ns: u64,
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
    /// > 0: allocation does not collect (`R_gc` still does).
    pub gc_inhibit: u32,
    /// a collection is running (see [`Core::root_fns`])
    collecting: bool,
    /// the bounds of the thread the core is bound to (the evaluator's)
    stack_top: usize,
    stack_lo: usize,
    /// the highest address the conservative scan reads: the thread's
    /// stack top by default; the embedder narrows it to the C segments
    /// (0: no stack scan) — see [`Core::set_scan_top`]
    scan_top: usize,
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

    /// R_PreserveObject's set, with counts
    /// R_PreserveObject's multiset, as GNU R's precious list: a stack,
    /// released from the top (O(1) for the usual LIFO churn)
    precious: Vec<Sexp>,
    /// Objects the host holds (the native bridge: core objects runtime
    /// values wrap, proxies pinned for C), by hold id. Roots, unless
    /// `held_roots` is off: then the host marks the ones it still reaches
    /// (through its own object graph, from [`Core::mark`] in a root hook or
    /// the host trace), and the others die with this collection, their
    /// ids queued in `dead_held` (one mark over both heaps, docs/core.md
    /// §6).
    held: HashMap<u64, Sexp, BuildFx>,
    next_held: u64,
    held_roots: bool,
    dead_held: Vec<u64>,
    /// Objects the host watches without rooting them ([`Core::watch`]):
    /// a collection that marks one makes it a hold (the pair of ids in
    /// `kept_watched`), one that does not frees it (its id in
    /// `lost_watched`). Checked between mark and sweep in every collection.
    watched: HashMap<u64, Sexp, BuildFx>,
    kept_watched: Vec<(u64, u64)>,
    lost_watched: Vec<u64>,
    /// extra precise roots: called during marking with this core.
    ///
    /// The contract of every hook here: the collector is running. A hook
    /// may call [`Core::mark`] and read objects, and nothing else on this
    /// core: it must not allocate or collect. An object allocated now is
    /// not marked and the sweep would free it; a collection nested in this
    /// one panics, which `ROBIGO_GCTORTURE` turns into a certainty for
    /// any hook that allocates.
    root_fns: Vec<unsafe fn(&mut Core)>,
    /// Strong runtime edges of a marked HOST object, visited during every
    /// tracing phase (including the weak-reference fixed point). A hook:
    /// see [`Core::root_fns`].
    host_trace: Option<unsafe fn(&mut Core, usize)>,
    /// Release temporary host graph owners before deferred proxy releases.
    /// Runs after the sweep, while the collection is still in progress: it
    /// may release preserved objects but must not allocate or collect.
    host_trace_end: Option<unsafe fn()>,
    /// weak references (WEAKREFSXP) not yet finalized
    weak_refs: Vec<Sexp>,
    /// weak references whose key died, waiting for their finalizer
    finalize_queue: Vec<Sexp>,
    /// release callbacks of objects found dead by the last sweep
    releases: Vec<(ReleaseFn, usize)>,
    /// immortal objects something was stored into (a symbol's value,
    /// attributes set on a shared constant): traced on every collection.
    /// Stores reach it through [`Core::write_barrier`].
    immortal_roots: Vec<Sexp>,
    immortal_seen: std::collections::HashSet<Sexp, BuildFx>,
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

/// The calling thread's stack: (lowest address, highest address).
#[cfg(target_vendor = "apple")]
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
#[cfg(not(target_vendor = "apple"))]
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
#[cfg(not(target_vendor = "apple"))]
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

impl Core {
    /// A new heap. The calling thread's stack is the one scanned
    /// conservatively.
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
            stack_top,
            stack_lo,
            scan_top: usize::MAX,
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
            held: HashMap::default(),
            next_held: 1,
            held_roots: true,
            dead_held: Vec::new(),
            watched: HashMap::default(),
            kept_watched: Vec::new(),
            lost_watched: Vec::new(),
            root_fns: Vec::new(),
            host_trace: None,
            host_trace_end: None,
            weak_refs: Vec::new(),
            finalize_queue: Vec::new(),
            releases: Vec::new(),
            immortal_roots: Vec::new(),
            immortal_seen: Default::default(),
        });
        // SAFETY: every object here is fresh from this heap, node-sized where
        // node fields are written, and flagged immortal before the next
        // allocation can collect, so none of them can be swept.
        unsafe {
            // R_NilValue: a node whose fields point at itself (memory.c InitMemory)
            let nil = c.alloc_object(NILSXP, NODE_SIZE, 0);
            nil.set_attrib_raw(nil);
            nil.set_car_raw(nil);
            nil.set_cdr_raw(nil);
            nil.set_tag_raw(nil);
            c.nil = nil;
            c.make_immortal(nil);
            nil.set_attrib_raw(nil);
            // NA_STRING is not in the cache (names.c: mkChar("NA") is a different object)
            let na = c.alloc_char(b"NA", 0, hash_bytes(b"NA"));
            c.make_immortal(na);
            c.na_string = na;
            let blank = c.mk_char(b"", 0);
            c.make_immortal(blank);
            c.blank_string = blank;
            // R_UnboundValue and R_MissingArg are symbols with empty names
            // that are not in the symbol table (names.c InitNames)
            c.unbound = c.new_symbol(blank);
            c.missing_arg = c.new_symbol(blank);
        }
        // like InitMemory: once the constants exist
        (c.force_wait, c.force_gap) = torture_from_env();
        c
    }

    /// Flag `x` immortal: never marked, never freed. Only for objects no
    /// other object can be traced through to anything mortal.
    ///
    /// # Safety
    /// `x` must be a live object of this heap. From here on the collector
    /// does not trace `x`: whatever it points at now must be immortal, or be
    /// kept alive some other way, until a store into `x` passes
    /// [`Core::write_barrier`], which makes `x` a root. An object flagged
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
            x.set_flag(flags::IMMORTAL, true);
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

    /// Run `f` during every collection, to mark the embedder's roots.
    ///
    /// # Safety
    /// `f` keeps the contract of a hook, documented on the field
    /// `Core::root_fns`: it may call [`Core::mark`] and read objects, and
    /// nothing else on this core.
    pub unsafe fn add_root_fn(&mut self, f: unsafe fn(&mut Core)) {
        self.root_fns.push(f);
    }

    /// Set the hooks for HOST objects: `trace` marks the strong runtime edges
    /// of one, and `end` releases temporary graph owners after a sweep.
    ///
    /// # Safety
    /// As for [`Core::add_root_fn`]; `trace` also accepts every handle given
    /// to [`Core::alloc_host`], and `end` may release preserved objects.
    pub unsafe fn set_host_trace(&mut self, trace: Option<unsafe fn(&mut Core, usize)>, end: Option<unsafe fn()>) {
        self.host_trace = trace;
        self.host_trace_end = end;
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

    /// Collect if one is due before allocating an object whose vector data
    /// takes `cells` VECRECs.
    // R: memory.c:allocVector3 / allocSExp: `if (FORCE_GC || NO_FREE_NODES()
    // || VHEAP_FREE() < alloc_size) R_gc_internal(alloc_size)`
    #[inline(always)]
    fn collect_if_due(&mut self, cells: usize) {
        if (!self.sizes.pending && self.force_wait > 0 && self.force_gc()) || self.sizes.due(cells) {
            self.gc_internal(cells);
        }
    }

    /// A collection is due: run it, or postpone it while collections are
    /// inhibited.
    // R: memory.c:R_gc_internal
    #[cold]
    #[inline(never)]
    fn gc_internal(&mut self, size_needed: usize) {
        if self.gc_inhibit > 0 {
            self.sizes.postpone(size_needed);
            return;
        }
        self.sizes.pending = false;
        self.collect_needing(size_needed);
    }

    /// Count a new object of `cells` VECRECs in the heap sizes.
    #[inline(always)]
    fn count(&mut self, bytes: usize, cells: usize) {
        self.alloc_since_gc += bytes;
        self.sizes.nodes_in_use += 1;
        self.sizes.valloc += cells;
    }

    /// A raw object of `size` bytes with a fresh header (type `t`, no
    /// flags, attributes R_NilValue), whose vector data GNU R would count
    /// as `cells` VECRECs ([`crate::heapsize::vec_cells`]; 0 for nodes).
    #[inline(always)]
    pub(crate) fn alloc_object(&mut self, t: u8, size: usize, cells: usize) -> Sexp {
        self.collect_if_due(cells);
        let size = size.max(16);
        let p = self.heap.alloc(size);
        // SAFETY: `p` has at least 16 bytes, 16-byte aligned, and no one else
        // has it yet. Writing +0 as a word clears the flags, gp and counts.
        unsafe {
            *(p as *mut u64) = t as u64;
            *((p as *mut *mut u8).add(1)) = self.nil.0;
        }
        self.count(size, cells);
        Sexp(p)
    }

    /// A node (cons cell shape): LISTSXP, LANGSXP, CLOSXP, ENVSXP, ...
    ///
    /// # Safety
    /// `car`, `cdr` and `tag` must be live objects of this heap, and stay
    /// rooted through this call, which can collect: they are reference
    /// counted here and traced by every later collection. Only for
    /// EXTPTRSXP, BUILTINSXP and SPECIALSXP may `car` be any word.
    #[inline]
    pub unsafe fn alloc_node(&mut self, t: u8, car: Sexp, cdr: Sexp, tag: Sexp) -> Sexp {
        let x = self.alloc_object(t, NODE_SIZE, 0);
        // SAFETY: `x` has node fields. The allocation above may collect, but
        // the caller's contract keeps `car`, `cdr` and `tag` live through it,
        // and only an object's count is written, never a non-object `car`.
        unsafe {
            x.set_car_raw(car);
            x.set_cdr_raw(cdr);
            x.set_tag_raw(tag);
            // GNU R's constructors count the children they store; an
            // external pointer's car is an address and a primitive's an
            // offset, not objects
            // (R_NilValue, the commonest child, is immortal: a pointer
            // comparison spares reading its flags)
            let nil = self.nil;
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
        x
    }

    /// A vector as GNU R's allocVector: STRSXP filled with
    /// R_BlankString, VECSXP/EXPRSXP with R_NilValue, others not
    /// initialized.
    ///
    /// # Panics
    /// If `t` is not a vector type (the check folds away for a constant `t`).
    #[inline]
    pub fn alloc_vector(&mut self, t: u8, n: usize) -> Sexp {
        let esz = crate::elt_size(t);
        if n <= 256 && !matches!(t, STRSXP | VECSXP | EXPRSXP | WEAKREFSXP) {
            if esz == 0 {
                not_a_vector_type(t);
            }
            // small atomic vectors: no overflow check, no fill
            let x = self.alloc_object(t, (OFF_DATA + n * esz).div_ceil(16) * 16, crate::heapsize::vec_cells(t, n));
            // SAFETY: at least OFF_DATA bytes were requested, and a fresh
            // vector of a type with elements has its length fields there.
            unsafe {
                x.set_std_len(n as isize);
                x.set_truelen(0);
            }
            return x;
        }
        self.alloc_vector_slow(t, n)
    }

    fn alloc_vector_slow(&mut self, t: u8, n: usize) -> Sexp {
        match self.try_alloc_vector(t, n) {
            Some(x) => x,
            None => panic!("robigo: cannot allocate a vector of {n} elements"),
        }
    }

    /// A vector, or None if there is no memory for it (after a
    /// collection): the API turns that into GNU R's "cannot allocate
    /// vector of size ..." error.
    ///
    /// # Panics
    /// If `t` is not a vector type.
    pub fn try_alloc_vector(&mut self, t: u8, n: usize) -> Option<Sexp> {
        let esz = crate::elt_size(t);
        if esz == 0 {
            not_a_vector_type(t);
        }
        let bytes = n.checked_mul(esz)?.checked_add(OFF_DATA)?.checked_add(15)? / 16 * 16;
        let cells = crate::heapsize::vec_cells(t, n);
        let x = if bytes > crate::heap::MAX_MEDIUM {
            self.collect_if_due(cells);
            let p = match self.heap.try_alloc_large(bytes) {
                Some(p) => p,
                None => {
                    if self.gc_inhibit == 0 {
                        self.collect();
                    }
                    self.heap.try_alloc_large(bytes)?
                }
            };
            // SAFETY: as in `alloc_object`: `p` is fresh, with `bytes` bytes.
            unsafe {
                *(p as *mut u64) = t as u64;
                *((p as *mut *mut u8).add(1)) = self.nil.0;
            }
            self.count(bytes, cells);
            Sexp(p)
        } else {
            self.alloc_object(t, bytes, cells)
        };
        // SAFETY: `x` is fresh with `OFF_DATA + n * esz` bytes (no overflow:
        // checked above), and for the filled types `esz` is a pointer's size,
        // so the `n` writes stay in the payload.
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
        Some(x)
    }

    /// A vector whose payload belongs to someone else: `release(owner)`
    /// is called once the object is dead (after the sweep).
    ///
    /// # Safety
    /// `data` must hold `n` elements of type `t` and stay valid until
    /// `release` runs; `release` must accept `owner` after any later sweep.
    pub unsafe fn alloc_extdata(
        &mut self,
        t: u8,
        data: *mut u8,
        n: usize,
        owner: usize,
        release: Option<ReleaseFn>,
    ) -> Sexp {
        // (the payload is not on this heap: a node, as GNU R counts a
        // custom allocator's vector)
        let x = self.alloc_object(t, EXTDATA_SIZE, 0);
        // SAFETY: EXTDATA_SIZE covers every field written, up to +56.
        unsafe {
            x.set_flags(flags::EXTDATA);
            x.set_std_len(n as isize);
            x.set_truelen(0);
            *(x.0.add(32) as *mut *mut u8) = data;
            *(x.0.add(40) as *mut usize) = owner;
            *(x.0.add(48) as *mut Option<ReleaseFn>) = release;
        }
        x
    }

    /// A proxy for a host object: `release(handle)` runs once it is dead.
    /// The node fields are R_NilValue (the API asks the host for them);
    /// the handle and release function follow at +40/+48.
    ///
    /// # Safety
    /// `release` may be called with `handle` from any collection, and
    /// [`Core::set_host_trace`]'s hook is too: both must accept it.
    pub unsafe fn alloc_host(&mut self, t: u8, handle: usize, release: Option<ReleaseFn>) -> Sexp {
        let x = self.alloc_object(t, crate::HOST_SIZE, 0);
        // SAFETY: HOST_SIZE covers every field written, up to +56.
        unsafe {
            x.set_flags(flags::HOST);
            x.set_car_raw(self.nil);
            x.set_cdr_raw(self.nil);
            x.set_tag_raw(self.nil);
            *(x.0.add(40) as *mut usize) = handle;
            *(x.0.add(48) as *mut Option<ReleaseFn>) = release;
        }
        x
    }

    /// The host handle of a HOST proxy.
    ///
    /// # Safety
    /// `x` must be a live object made by [`Core::alloc_host`] (flag HOST).
    /// Any other object has something else at +40, or nothing: it can be
    /// past the end of the object.
    pub unsafe fn host_handle(x: Sexp) -> usize {
        // SAFETY: the caller's contract: a HOST object has its handle at +40.
        unsafe { *(x.0.add(40) as *const usize) }
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

    // ---- CHARSXPs and symbols -------------------------------------------

    fn alloc_char(&mut self, bytes: &[u8], gpbits: u16, h: u32) -> Sexp {
        let x = self.alloc_object(
            CHARSXP,
            (OFF_DATA + bytes.len() + 1).div_ceil(16) * 16,
            crate::heapsize::vec_cells(CHARSXP, bytes.len()),
        );
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
        x
    }

    /// The cached CHARSXP for `bytes` with encoding bits `encbits` (a
    /// combination of `gp::{UTF8, LATIN1, BYTES}`; ASCII is computed here
    /// and replaces the others, as mkCharLenCE does). The caller has
    /// rejected embedded NULs.
    pub fn mk_char(&mut self, bytes: &[u8], encbits: u16) -> Sexp {
        let ascii = bytes.is_ascii();
        let bits = if ascii { gp::ASCII } else { encbits & (gp::UTF8 | gp::LATIN1 | gp::BYTES) } | gp::CACHED;
        let h = hash_bytes(bytes);
        let enc = bits & !gp::CACHED;
        // SAFETY: the table's entries are live CHARSXPs: the collector removes
        // dead ones before it frees them.
        let found = self.chars.find(h, |c| unsafe {
            c.gp() & (gp::UTF8 | gp::LATIN1 | gp::BYTES | gp::ASCII) == enc && c.char_bytes() == bytes
        });
        if let Some(c) = found {
            return c;
        }
        let c = self.alloc_char(bytes, bits, h);
        // SAFETY: `c` is a new CHARSXP of this heap, and this table is its.
        unsafe { self.chars.insert(c) };
        c
    }

    fn new_symbol(&mut self, name: Sexp) -> Sexp {
        let s = self.alloc_object(SYMSXP, NODE_SIZE, 0);
        // SAFETY: `s` is fresh and node-sized, and no allocation follows, so
        // `name` (immortal, in both callers) and `nil` cannot be swept first.
        unsafe {
            s.set_car_raw(name);
            s.set_cdr_raw(self.unbound_or(s));
            s.set_tag_raw(self.nil);
            self.make_immortal(s);
        }
        s
    }

    fn unbound_or(&self, s: Sexp) -> Sexp {
        if self.unbound.is_null() { s } else { self.unbound }
    }

    /// The symbol named `name` (GNU R's install).
    pub fn install(&mut self, name: &[u8]) -> Sexp {
        if let Some(&s) = self.syms.get(name) {
            return s;
        }
        let c = self.mk_char(name, 0);
        // SAFETY: `c` is live, and a CHARSXP has no children.
        unsafe { self.make_immortal(c) };
        let s = self.new_symbol(c);
        self.syms.insert(name.into(), s);
        s
    }

    /// The symbol for a CHARSXP's bytes, or None if not installed.
    pub fn find_symbol(&self, name: &[u8]) -> Option<Sexp> {
        self.syms.get(name).copied()
    }

    /// How many CHARSXPs the cache holds, dead ones not yet pruned included.
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

    /// Hold `x` for the host; the id releases it ([`Core::unhold`]) and
    /// names it in [`Core::take_dead_held`].
    ///
    /// # Safety
    /// `x` must be a live object of this heap: while `held_roots` is on
    /// (the default) every collection traces it until it is released.
    pub unsafe fn hold(&mut self, x: Sexp) -> u64 {
        let id = self.next_held;
        self.next_held += 1;
        self.held.insert(id, x);
        id
    }

    /// Release a hold (no effect on an id whose object has died).
    pub fn unhold(&mut self, id: u64) {
        self.held.remove(&id);
    }

    /// Whether held objects are roots of the next collections; returns
    /// the previous setting. With it off, the host marks what it still
    /// holds itself, and the rest is collected.
    pub fn set_held_roots(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.held_roots, on)
    }

    /// The holds whose objects died in the collections since the last
    /// call: they are released already, and the objects freed.
    pub fn take_dead_held(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.dead_held)
    }

    /// Watch `x` without rooting it: the next collection that marks it
    /// (something else keeps it: the precious list, a C root, an object
    /// that is kept) turns the watch into a hold, and one that does not
    /// lets it die. [`Core::take_watched`] reports both.
    ///
    /// # Safety
    /// `x` must be a live object of this heap.
    pub unsafe fn watch(&mut self, x: Sexp) -> u64 {
        let id = self.next_held;
        self.next_held += 1;
        self.watched.insert(id, x);
        id
    }

    /// Stop watching (no effect on an id a collection has decided).
    pub fn unwatch(&mut self, id: u64) {
        self.watched.remove(&id);
    }

    /// The watches the collections since the last call decided: (watch id,
    /// the id of the hold it became) for the kept, and the ids of the lost.
    pub fn take_watched(&mut self) -> (Vec<(u64, u64)>, Vec<u64>) {
        (std::mem::take(&mut self.kept_watched), std::mem::take(&mut self.lost_watched))
    }

    /// How many objects are held.
    pub fn held_count(&self) -> usize {
        self.held.len()
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

    /// Release callbacks of host proxies and external buffers the last
    /// collections found dead. They never run inside a collection; the
    /// API runs them at safe points.
    pub fn take_releases(&mut self) -> Vec<(ReleaseFn, usize)> {
        std::mem::take(&mut self.releases)
    }

    /// Limit the conservative stack scan to addresses below `top` (the
    /// stack grows down: frames entered after `top` was taken). `0` turns
    /// the scan off, `usize::MAX` scans to the thread's stack top. An
    /// embedder whose own frames hold no core objects except precisely
    /// rooted ones (robigo's runtime before G3) scans only the frames of
    /// C code it called, which keeps collections cheap and finalizer
    /// timing deterministic. Returns the previous limit.
    pub fn set_scan_top(&mut self, top: usize) -> usize {
        std::mem::replace(&mut self.scan_top, top)
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

    /// Collect on every allocation (`gctorture(on)`).
    pub fn set_torture(&mut self, on: bool) {
        self.gc_torture(on as i32, 0);
    }

    /// The torture counters: (allocations until the next forced
    /// collection, allocations between them); (0, 0) when off.
    pub fn torture_counters(&self) -> (i32, i32) {
        (self.force_wait, self.force_gap)
    }

    /// Collect every `gap` allocations, the first after `wait` of them
    /// (`gap` 0 turns torture off; negative or NA values change nothing).
    // R: memory.c:R_gc_torture (without PROTECTCHECK: no inhibit_release)
    pub fn gc_torture(&mut self, gap: i32, wait: i32) {
        if gap >= 0 {
            self.force_wait = gap;
            self.force_gap = gap;
        }
        if gap > 0 && wait > 0 {
            self.force_wait = wait;
        }
    }

    /// Weak references still waiting for their key to die (for
    /// finalizers that run on exit).
    pub fn weak_refs(&self) -> &[Sexp] {
        &self.weak_refs
    }

    /// Every field of every weak reference the collector still holds (key,
    /// value, finalizer), for both entries awaiting classification and ones
    /// queued for RunFinalizers. GNU R keeps key, value and finalizer alive
    /// for the finalization round (memory.c RunGenCollect), so the runtime
    /// must not sweep what only they reach yet.
    pub fn weak_ref_fields(&self) -> Vec<Sexp> {
        self.weak_refs
            .iter()
            .chain(self.finalize_queue.iter())
            // SAFETY: both lists hold initialized WEAKREFSXPs that every
            // collection marks.
            .flat_map(|&w| (0..3).map(move |i| unsafe { Self::weak_field(w, i) }))
            .collect()
    }

    /// The store barrier: every store of an object into a field of `x`
    /// goes through here (the API's setters do). Immortal objects are
    /// never marked, so one that received a store becomes a root traced
    /// on every collection (a symbol's value, attributes on a shared
    /// constant). The hook a generational collector extends (docs/core.md
    /// §11: sticky marks).
    ///
    /// # Safety
    /// `x` must be a live object of this heap: its header is read here, and
    /// an immortal `x` becomes a root of every later collection.
    #[inline(always)]
    pub unsafe fn write_barrier(&mut self, x: Sexp) {
        // SAFETY: the caller's contract: `x` is live.
        if unsafe { x.has_flag(flags::IMMORTAL) } {
            // SAFETY: as above, and immortal, so `x` is never freed.
            unsafe { self.remember_immortal(x) };
        }
    }

    /// # Safety
    /// `x` must be an immortal object of this heap: it is read here, and on
    /// every collection from now on.
    #[cold]
    unsafe fn remember_immortal(&mut self, x: Sexp) {
        // A CHARSXP has nothing to trace, and must not be: `trace` would
        // count it in `marked_chars` on top of `immortal_chars`, and with
        // the sum inflated `prune_chars` could skip a table with dead entries.
        // SAFETY: the caller's contract: `x` is live.
        if unsafe { x.typ() } == CHARSXP {
            return;
        }
        if self.immortal_seen.insert(x) {
            self.immortal_roots.push(x);
        }
    }

    /// Mark `x` and everything reachable from it (root callbacks call
    /// this).
    ///
    /// # Safety
    /// `x` must be null or a live object of this heap.
    #[inline]
    pub unsafe fn mark(&mut self, x: Sexp) {
        // SAFETY: `mark_obj` asks for what this function's contract gives.
        unsafe { self.mark_obj(x) }
    }

    /// Bytes reserved from the OS.
    pub fn heap_reserved(&self) -> usize {
        self.heap.reserved
    }

    /// Whether `x` points into an object of this heap (start or interior).
    pub fn resolve(&self, w: usize) -> Option<Sexp> {
        self.heap.resolve(w).map(Sexp)
    }

    /// Whether `w` lies in memory this heap reserved (an object, free
    /// space or a block header): no claim that an object starts there.
    pub fn in_heap(&self, w: usize) -> bool {
        self.heap.chunk_of(w).is_some()
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
    /// fields hold live objects (the rules of the raw setters).
    unsafe fn trace(&mut self) {
        // SAFETY: the caller's contract. Each body is read as what its type
        // and flags say it is, and the order matters: a CHARSXP has its hash
        // where attributes go, an ALTREP object of a vector type has no
        // length, and a HOST object's fields are the host's.
        unsafe {
            while let Some(p) = self.mark_stack.pop() {
                let t = p.typ();
                if t == CHARSXP {
                    // every cached CHARSXP is in the table (mk_char)
                    self.marked_chars += (p.gp() & gp::CACHED != 0) as usize;
                    continue;
                }
                self.mark_obj(p.attrib_raw());
                let f = p.flags();
                if f & flags::HOST != 0 {
                    if let Some(trace) = self.host_trace {
                        trace(self, Self::host_handle(p));
                    }
                    continue;
                }
                if f & flags::ALTREP != 0 {
                    self.mark_obj(p.car());
                    self.mark_obj(p.cdr());
                    self.mark_obj(p.tag());
                    continue;
                }
                match t {
                    STRSXP | VECSXP | EXPRSXP => {
                        let n = p.std_len() as usize;
                        let d = p.std_data() as *const Sexp;
                        for i in 0..n {
                            self.mark_obj(*d.add(i));
                        }
                    }
                    // fields are handled by the weak-reference pass
                    WEAKREFSXP => {}
                    EXTPTRSXP => {
                        self.mark_obj(p.cdr());
                        self.mark_obj(p.tag());
                    }
                    LISTSXP | LANGSXP | DOTSXP | CLOSXP | ENVSXP | PROMSXP | BCODESXP | OBJSXP | SYMSXP => {
                        self.mark_obj(p.car());
                        self.mark_obj(p.cdr());
                        self.mark_obj(p.tag());
                    }
                    _ => {}
                }
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

    /// Mark what the machine stack points at, from this frame up.
    ///
    /// # Safety
    /// `ctx` must be the core `collect` passed to `robigo_spill`, the only
    /// way to reach it during this call, and the thread must be the core's.
    unsafe extern "C" fn scan_stack(ctx: *mut c_void) {
        // SAFETY: `ctx` is as the caller promises. [sp, top) is mapped stack
        // of this thread (collect checked the thread) and word-aligned, and
        // `resolve` only returns allocated objects. This is the one block in
        // the crate whose reason lies outside Rust's rules: the loop reads
        // other frames' memory, which can be uninitialized, or borrowed
        // exclusively by a frame that is suspended here. A conservative scan
        // needs those reads. It works because they are ordinary machine
        // loads of integers compared to addresses, in a function only called
        // through C, so nothing can be optimized from them.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let top = c.stack_top.min(c.scan_top);
            let (min, max) = (c.heap.min_addr, c.heap.max_addr);
            let mut w = (sp + 7) & !7;
            while w < top {
                let v = *(w as *const usize);
                if v >= min && v < max {
                    if let Some(p) = c.heap.resolve(v) {
                        c.mark_obj(Sexp(p));
                    }
                    // a word on a granule boundary may be one past the end
                    // of the previous object (a loop bound, p + n)
                    if v & 15 == 0 {
                        if let Some(p) = c.heap.resolve(v - 1) {
                            c.mark_obj(Sexp(p));
                        }
                    }
                }
                w += 8;
            }
            c.trace();
        }
    }

    /// A full collection. Returns the pause in nanoseconds. Release
    /// callbacks of dead EXTDATA/HOST objects run at the end; queued
    /// finalizers do not run here (see [`Core::pop_finalizable`]).
    ///
    /// # Panics
    /// If a collection is already running: a GC hook allocated or collected
    /// (see [`Core::root_fns`]).
    pub fn collect(&mut self) -> u64 {
        // R: memory.c:R_gc -> R_gc_internal(0)
        self.sizes.pending = false;
        self.collect_needing(0)
    }

    /// A full collection before allocating `size_needed` VECRECs; the heap
    /// sizes are adjusted for it afterwards.
    // R: memory.c:R_gc_internal (RunGenCollect, then AdjustHeapSize after a
    // collection of every generation: here each one is)
    fn collect_needing(&mut self, size_needed: usize) -> u64 {
        assert!(!self.collecting, "robigo: a GC hook allocated or collected inside a collection");
        self.collecting = true;
        self.stats.bytes_allocated += self.alloc_since_gc as u64;
        let sp = stack_pointer();
        if sp < self.stack_lo || sp > self.stack_top {
            eprintln!("robigo: the R heap was used from a thread other than the R thread");
            std::process::abort();
        }
        let t0 = Instant::now();
        self.epoch += 1;
        self.marked_chars = 0;
        // SAFETY: no object was freed since the last sweep, so every list
        // here holds objects whose fields are objects (the rooting contracts
        // of `preserve`, `register_weak_ref` and `write_barrier`). The hooks
        // are called as registered, on this core, while marking; `spill`
        // gets a callback and a context of matching types; and this thread
        // is the core's, checked above.
        unsafe {
            for i in 0..self.precious.len() {
                let p = self.precious[i];
                self.mark_obj(p);
            }
            if self.held_roots {
                let held: Vec<Sexp> = self.held.values().copied().collect();
                for p in held {
                    self.mark_obj(p);
                }
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
            // change (a symbol's value, attributes set on a constant)
            for i in 0..self.immortal_roots.len() {
                let x = self.immortal_roots[i];
                self.mark_stack.push(x);
            }
            let fns = std::mem::take(&mut self.root_fns);
            for f in &fns {
                f(self);
            }
            self.root_fns = fns;
            self.trace();
            crate::unwind::robigo_spill(Self::scan_stack, self as *mut Core as *mut c_void);
            self.process_weak_refs();
            self.prune_chars();
            if !self.held_roots {
                // nothing marked them: the host's hold dies with them
                let dead: Vec<u64> =
                    self.held.iter().filter(|&(_, &x)| !self.is_marked(x)).map(|(&id, _)| id).collect();
                for id in &dead {
                    self.held.remove(id);
                }
                self.dead_held.extend(dead);
            }
            // what the host watches: kept by something else, or lost
            // (decided before the sweep frees the lost and reuses their
            // slots; a kept one is held from here on)
            if !self.watched.is_empty() {
                let watched: Vec<(u64, Sexp)> = self.watched.drain().collect();
                for (id, x) in watched {
                    if self.is_marked(x) {
                        let h = self.next_held;
                        self.next_held += 1;
                        self.held.insert(h, x);
                        self.kept_watched.push((id, h));
                    } else {
                        self.lost_watched.push(id);
                    }
                }
            }
            self.sweep();
            if let Some(end) = self.host_trace_end {
                end();
            }
        }
        self.sizes.adjust(size_needed);
        self.collecting = false;
        let dt = t0.elapsed().as_nanos() as u64;
        self.stats.collections += 1;
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
        // by `alloc_extdata` or `alloc_host`, which fill +40 and +48, and +48
        // is an `Option<ReleaseFn>`: API code that disarms it writes null.
        unsafe {
            // R: memory.c:ReleaseLargeFreeVectors (R_LargeVallocSize -=
            // getVecSizeInVEC(s)); small vectors are counted by class there,
            // which is the same number
            self.sizes.valloc = self.sizes.valloc.saturating_sub(crate::heapsize::object_cells(Sexp(p)));
            let f = *p.add(1);
            if f & (flags::EXTDATA | flags::HOST) != 0 {
                // EXTDATA and HOST objects both keep owner/handle at +40
                // and the release function at +48
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
                // SAFETY: a large chunk holds one object at DATA_OFF, and it
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
            // SAFETY: `to_list` holds blocks found above in chunks that are
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

/// [`Core::heap_sizes`]: the collection trigger's state in GNU R's units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapReport {
    /// `R_NSize`: nodes the heap may hold before a collection.
    pub n_size: usize,
    /// `R_VSize`: the vector heap in VECRECs.
    pub v_size: usize,
    /// Nodes in use.
    pub nodes_in_use: usize,
    /// VECRECs in use (`R_VSize` counts against these).
    pub vecrecs_in_use: usize,
}
