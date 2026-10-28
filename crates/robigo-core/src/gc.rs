//! The collector and the allocation entry points: a non-moving
//! mark-region heap (blocks, size classes: `heap.rs`) with precise roots
//! (protect stack, precious set, registered root callbacks, weak
//! references) and a conservative scan of the machine stack. Ported from
//! the bake-off's `mm-gc` (`experiments/memory`), which measured this
//! design against reference counting (`docs/ledger/memory.md`).

use crate::heap::{BLOCK, Block, CLASS_NONE, DATA_OFF, Heap, OWNER_EMPTY, OWNER_NONE, SIZES, START_WORDS};
use crate::types::*;
use crate::{EXTDATA_SIZE, NODE_SIZE, OFF_DATA, Sexp, flags, gp, hash_bytes};
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Instant;

/// Releases an external payload or host handle when its object dies.
pub type ReleaseFn = unsafe extern "C" fn(usize);

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

/// The collection floor: grow at least this much between collections
/// (2 MiB: the bake-off's locality knee). `ROBIGO_GC_MIN` (bytes)
/// overrides it so tests can stress tiny heaps.
fn gc_min() -> usize {
    static V: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("ROBIGO_GC_MIN").ok().and_then(|s| s.parse().ok()).unwrap_or(2 << 20))
}

/// One heap and its tables. Single-threaded, like R's values.
pub struct Core {
    heap: Heap,
    mark_stack: Vec<Sexp>,
    cur_mark: u8,
    alloc_since_gc: usize,
    gc_threshold: usize,
    /// > 0: allocation does not collect (`R_gc` still does).
    pub gc_inhibit: u32,
    stack_top: usize,
    pub stats: GcStats,

    // the immortal constants
    pub nil: Sexp,
    pub na_string: Sexp,
    pub blank_string: Sexp,
    pub unbound: Sexp,
    pub missing_arg: Sexp,

    /// the CHARSXP cache: weak, pruned after marking
    chars: HashMap<u32, Vec<Sexp>, BuildFx>,
    /// symbols by name (immortal)
    syms: HashMap<Box<[u8]>, Sexp, BuildFx>,

    /// R_PreserveObject's set, with counts
    precious: HashMap<Sexp, u32, BuildFx>,
    /// extra precise roots: called during marking with this core
    pub root_fns: Vec<unsafe fn(&mut Core)>,
    /// weak references (WEAKREFSXP) not yet finalized
    weak_refs: Vec<Sexp>,
    /// weak references whose key died, waiting for their finalizer
    finalize_queue: Vec<Sexp>,
    /// release callbacks of objects found dead by the last sweep
    releases: Vec<(ReleaseFn, usize)>,
    /// immortal objects whose fields can still change (symbols' values,
    /// attributes set on shared constants): traced on every collection
    immortal_roots: Vec<Sexp>,
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

/// The highest address of the calling thread's stack.
fn thread_stack_top() -> usize {
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return 0;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        addr as usize + size
    }
}

impl Core {
    /// A new heap. The calling thread's stack is the one scanned
    /// conservatively.
    pub fn new() -> Box<Core> {
        let mut c = Box::new(Core {
            heap: Heap::new(),
            mark_stack: Vec::new(),
            cur_mark: flags::MARK_B, // the first collection flips to MARK_A
            alloc_since_gc: 0,
            gc_threshold: gc_min(),
            gc_inhibit: 0,
            stack_top: thread_stack_top(),
            stats: GcStats::default(),
            nil: Sexp::NULL,
            na_string: Sexp::NULL,
            blank_string: Sexp::NULL,
            unbound: Sexp::NULL,
            missing_arg: Sexp::NULL,
            chars: HashMap::default(),
            syms: HashMap::default(),
            precious: HashMap::default(),
            root_fns: Vec::new(),
            weak_refs: Vec::new(),
            finalize_queue: Vec::new(),
            releases: Vec::new(),
            immortal_roots: Vec::new(),
        });
        unsafe {
            // R_NilValue: a node whose fields point at itself (memory.c InitMemory)
            let nil = c.alloc_object(NILSXP, NODE_SIZE);
            nil.set_attrib_raw(nil);
            nil.set_car_raw(nil);
            nil.set_cdr_raw(nil);
            nil.set_tag_raw(nil);
            c.nil = nil;
            c.make_immortal(nil);
            nil.set_attrib_raw(nil);
            // NA_STRING is not in the cache (names.c: mkChar("NA") is a different object)
            let na = c.alloc_char(b"NA", 0);
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
        c
    }

    /// Flag `x` immortal: never marked, never freed. Only for objects no
    /// other object can be traced through to anything mortal.
    pub unsafe fn make_immortal(&mut self, x: Sexp) {
        unsafe {
            if x.has_flag(flags::IMMORTAL) {
                return;
            }
            x.set_flag(flags::IMMORTAL, true);
            if x.typ() != CHARSXP {
                self.immortal_roots.push(x);
            }
            let b = self.heap.block_of(x.0);
            if (*b).magic == 0x524f4249474f4241 {
                (*b).immortals += 1;
            }
            // large objects: the sweeper keeps them by the flag
        }
    }

    // ---- allocation ---------------------------------------------------

    /// A raw object of `size` bytes with a fresh header (type `t`, no
    /// flags, attributes R_NilValue).
    #[inline]
    pub fn alloc_object(&mut self, t: u8, size: usize) -> Sexp {
        if self.alloc_since_gc >= self.gc_threshold && self.gc_inhibit == 0 {
            self.collect();
        }
        let size = size.max(16);
        let p = self.heap.alloc(size);
        unsafe {
            *(p as *mut u64) = t as u64;
            *((p as *mut *mut u8).add(1)) = self.nil.0;
        }
        self.alloc_since_gc += size;
        self.stats.allocs += 1;
        self.stats.bytes_allocated += size as u64;
        Sexp(p)
    }

    /// A node (cons cell shape): LISTSXP, LANGSXP, CLOSXP, ENVSXP, ...
    #[inline]
    pub fn alloc_node(&mut self, t: u8, car: Sexp, cdr: Sexp, tag: Sexp) -> Sexp {
        let x = self.alloc_object(t, NODE_SIZE);
        unsafe {
            x.set_car_raw(car);
            x.set_cdr_raw(cdr);
            x.set_tag_raw(tag);
            if t != EXTPTRSXP {
                car.incref();
            }
            cdr.incref();
            tag.incref();
        }
        x
    }

    /// A vector as GNU R's allocVector: STRSXP filled with
    /// R_BlankString, VECSXP/EXPRSXP with R_NilValue, others not
    /// initialized.
    pub fn alloc_vector(&mut self, t: u8, n: usize) -> Sexp {
        let esz = crate::elt_size(t);
        let bytes = n.checked_mul(esz).and_then(|b| b.checked_add(OFF_DATA)).expect("vector too large");
        let x = self.alloc_object(t, bytes.div_ceil(16) * 16);
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
        x
    }

    /// A vector whose payload belongs to someone else: `release(owner)`
    /// is called once the object is dead (after the sweep).
    pub fn alloc_extdata(&mut self, t: u8, data: *mut u8, n: usize, owner: usize, release: Option<ReleaseFn>) -> Sexp {
        let x = self.alloc_object(t, EXTDATA_SIZE);
        unsafe {
            x.set_flags(flags::EXTDATA);
            x.set_std_len(n as isize);
            x.set_truelen(0);
            *(x.0.add(32) as *mut *mut u8) = data;
            *(x.0.add(40) as *mut usize) = owner;
            *(x.0.add(48) as *mut usize) = release.map_or(0, |f| f as usize);
        }
        x
    }

    /// A proxy for a host object: `release(handle)` runs once it is dead.
    pub fn alloc_host(&mut self, t: u8, handle: usize, release: Option<ReleaseFn>) -> Sexp {
        let x = self.alloc_object(t, NODE_SIZE);
        unsafe {
            x.set_flags(flags::HOST);
            *(x.0.add(16) as *mut usize) = handle;
            *(x.0.add(24) as *mut usize) = release.map_or(0, |f| f as usize);
            x.set_tag_raw(self.nil);
        }
        x
    }

    /// The host handle of a HOST proxy.
    pub unsafe fn host_handle(x: Sexp) -> usize {
        unsafe { *(x.0.add(16) as *const usize) }
    }

    /// The owner word of an EXTDATA vector.
    pub unsafe fn extdata_owner(x: Sexp) -> usize {
        unsafe { *(x.0.add(40) as *const usize) }
    }

    // ---- CHARSXPs and symbols -------------------------------------------

    fn alloc_char(&mut self, bytes: &[u8], gpbits: u16) -> Sexp {
        let x = self.alloc_object(CHARSXP, (OFF_DATA + bytes.len() + 1).div_ceil(16) * 16);
        unsafe {
            *(x.0.add(8) as *mut u64) = hash_bytes(bytes) as u64;
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
        if let Some(bucket) = self.chars.get(&h) {
            for &c in bucket {
                unsafe {
                    if c.gp() & (gp::UTF8 | gp::LATIN1 | gp::BYTES | gp::ASCII) == bits & !gp::CACHED
                        && c.char_bytes() == bytes
                    {
                        return c;
                    }
                }
            }
        }
        let c = self.alloc_char(bytes, bits);
        self.chars.entry(h).or_default().push(c);
        c
    }

    fn new_symbol(&mut self, name: Sexp) -> Sexp {
        let s = self.alloc_object(SYMSXP, NODE_SIZE);
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
        unsafe { self.make_immortal(c) };
        let s = self.new_symbol(c);
        self.syms.insert(name.into(), s);
        s
    }

    /// The symbol for a CHARSXP's bytes, or None if not installed.
    pub fn find_symbol(&self, name: &[u8]) -> Option<Sexp> {
        self.syms.get(name).copied()
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

    pub fn preserve(&mut self, x: Sexp) {
        *self.precious.entry(x).or_insert(0) += 1;
    }

    pub fn release(&mut self, x: Sexp) {
        if let Some(n) = self.precious.get_mut(&x) {
            *n -= 1;
            if *n == 0 {
                self.precious.remove(&x);
            }
        }
    }

    /// Register a weak reference object (a WEAKREFSXP: key, value,
    /// finalizer, next) with the collector.
    pub fn register_weak_ref(&mut self, w: Sexp) {
        self.weak_refs.push(w);
    }

    /// Weak references whose keys died, ready for their finalizers, in
    /// registration order. They stay reachable until taken.
    pub fn take_finalizable(&mut self) -> Vec<Sexp> {
        std::mem::take(&mut self.finalize_queue)
    }

    /// Weak references still waiting for their key to die (for
    /// finalizers that run on exit).
    pub fn weak_refs(&self) -> &[Sexp] {
        &self.weak_refs
    }

    /// Mark `x` and everything reachable from it (root callbacks call
    /// this).
    #[inline]
    pub fn mark(&mut self, x: Sexp) {
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

    // ---- collection ----------------------------------------------------

    #[inline]
    unsafe fn mark_obj(&mut self, p: Sexp) {
        unsafe {
            if p.is_null() {
                return;
            }
            let f = p.flags();
            if f & flags::IMMORTAL != 0 {
                return;
            }
            let cur = self.cur_mark;
            if f & cur != 0 {
                return;
            }
            p.set_flags((f & !(flags::MARK_A | flags::MARK_B)) | cur);
            let b = (p.0 as usize & !(BLOCK - 1)) as *mut Block;
            if (*b).magic == 0x524f4249474f4241 {
                (*b).marked = (*b).marked.wrapping_add(1);
            }
            self.mark_stack.push(p);
        }
    }

    #[inline]
    unsafe fn is_marked(&self, p: Sexp) -> bool {
        unsafe { p.flags() & (self.cur_mark | flags::IMMORTAL) != 0 }
    }

    unsafe fn trace(&mut self) {
        unsafe {
            while let Some(p) = self.mark_stack.pop() {
                let t = p.typ();
                if t == CHARSXP {
                    continue;
                }
                self.mark_obj(p.attrib_raw());
                let f = p.flags();
                if f & flags::HOST != 0 {
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

    unsafe fn weak_field(w: Sexp, i: usize) -> Sexp {
        unsafe { *(w.std_data() as *const Sexp).add(i) }
    }

    /// GNU R's weak-reference protocol (memory.c RunGenCollect): values
    /// and finalizers of references with live keys are live; references
    /// whose key died and that have a finalizer are queued, keeping key,
    /// value and finalizer alive for the finalizer; the others are
    /// cleared.
    unsafe fn process_weak_refs(&mut self) {
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
                if !self.is_marked(key) {
                    w.set_gp(w.gp() | gp::READY_TO_FINALIZE);
                    self.finalize_queue.push(w);
                } else {
                    keep.push(w);
                }
                for k in 0..3 {
                    self.mark_obj(Self::weak_field(w, k));
                }
            }
            self.trace();
            self.weak_refs = keep;
        }
    }

    unsafe extern "C" fn scan_stack(ctx: *mut c_void) {
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let top = c.stack_top;
            let (min, max) = (c.heap.min_addr, c.heap.max_addr);
            let mut w = (sp + 7) & !7;
            while w < top {
                let v = *(w as *const usize);
                if v >= min && v < max {
                    if let Some(p) = c.heap.resolve(v) {
                        c.mark_obj(Sexp(p));
                    }
                }
                w += 8;
            }
            c.trace();
        }
    }

    /// A full collection. Returns the pause in nanoseconds. Release
    /// callbacks of dead EXTDATA/HOST objects run at the end; queued
    /// finalizers do not run here (see [`Core::take_finalizable`]).
    pub fn collect(&mut self) -> u64 {
        let t0 = Instant::now();
        self.cur_mark = if self.cur_mark == flags::MARK_A { flags::MARK_B } else { flags::MARK_A };
        unsafe {
            let prec: Vec<Sexp> = self.precious.keys().copied().collect();
            for p in prec {
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
            self.sweep();
        }
        let dt = t0.elapsed().as_nanos() as u64;
        self.stats.collections += 1;
        self.stats.collect_total_ns += dt;
        self.stats.collect_max_pause_ns = self.stats.collect_max_pause_ns.max(dt);
        for (f, owner) in std::mem::take(&mut self.releases) {
            unsafe { f(owner) };
        }
        dt
    }

    fn prune_chars(&mut self) {
        let live = flags::IMMORTAL | self.cur_mark;
        self.chars.retain(|_, bucket| {
            bucket.retain(|&c| unsafe { c.flags() } & live != 0);
            !bucket.is_empty()
        });
    }

    /// Queue the release callback of a dead object.
    #[inline]
    unsafe fn on_death(&mut self, p: *mut u8) {
        unsafe {
            let f = *p.add(1);
            if f & (flags::EXTDATA | flags::HOST) != 0 {
                let (owner, rel) = if f & flags::EXTDATA != 0 {
                    (*(p.add(40) as *const usize), *(p.add(48) as *const usize))
                } else {
                    (*(p.add(16) as *const usize), *(p.add(24) as *const usize))
                };
                if rel != 0 {
                    self.releases.push((std::mem::transmute::<usize, ReleaseFn>(rel), owner));
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
        let cur = self.cur_mark;
        for (idx, c) in chunks.iter().enumerate() {
            if c.large {
                let obj = (c.base + DATA_OFF) as *mut u8;
                let f = unsafe { *obj.add(1) };
                if f & (cur | flags::IMMORTAL) != 0 {
                    live_bytes += (c.end - c.base) as u64;
                    live_objects += 1;
                } else {
                    unsafe { self.on_death(obj) };
                    dead_chunks.push(idx);
                }
                continue;
            }
            let mut chunk_dead = true;
            let mut chunk_touched = false;
            for bi in 0..c.claimed as usize {
                let b = (c.base + bi * BLOCK) as *mut Block;
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
                        chunk_dead = false;
                        continue;
                    }
                    let old_starts = (*b).starts;
                    (*b).starts = [0; START_WORDS];
                    (*b).freelist = std::ptr::null_mut();
                    let mut live: u32 = 0;
                    let csize = SIZES[(*b).class as usize] as u64;
                    for (w, &word) in old_starts.iter().enumerate() {
                        let mut s = word;
                        while s != 0 {
                            let g = w * 64 + s.trailing_zeros() as usize;
                            let p = (*b).data_ptr().add(g * 16);
                            let f = *p.add(1);
                            if f & (cur | flags::IMMORTAL) != 0 {
                                live += 1;
                                live_bytes += csize;
                                (*b).start_set(g);
                            } else {
                                self.on_death(p);
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
                    if !(*b).freelist.is_null() && (*b).owner == OWNER_NONE {
                        to_list.push(((*b).class as usize, b));
                    }
                }
            }
            if chunk_dead && chunk_touched {
                dead_small_chunks.push((idx, c.base, c.end));
            }
        }
        for (ci, b) in to_list {
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
        self.gc_threshold = ((live_bytes as usize) * 3 / 2).max(gc_min());
    }
}
