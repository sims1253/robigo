//! Variant (a): reference counting with a trial-deletion cycle collector
//! (Bacon & Rajan, "Concurrent Cycle Collection in Reference Counted
//! Systems", synchronous algorithm; the same shape CPython's gc uses).
//!
//! Layout: every object starts with the 16-byte header from
//! `docs/architecture.md` (type, flags, gp, refcnt, attributes) plus a
//! 4-byte `gc_refs` scratch count (padded to 24) that trial deletion
//! borrows while it runs. Handles own references: `Clone` increments,
//! `Drop` decrements. Refcounts saturate (GNU R does the same with
//! NAMEDMAX/REFCNT_MAX; saturated objects are effectively immortal).
//!
//! Cycles are found with the classic three phases over the subgraph
//! reachable from "purple" roots — objects whose refcount dropped but did
//! not reach zero: (1) snapshot refcounts into `gc_refs` and subtract all
//! internal edges; (2) scan from the roots: any object whose snapshot is
//! still positive is externally anchored and paints its reachable set
//! black; the rest turn white; (3) free the whites (their child references
//! are released by ordinary decrements, so shared children that are still
//! live keep correct counts). CHARSXPs are interned with the table holding
//! one reference; when the count drops to that one, the entry dies.

#![allow(unsafe_op_in_unsafe_fn)]

use mm_core::sexp::*;
use mm_core::{Mem, Stats, flags};
use mm_heap::{Heap, MAX_SMALL};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant;

const SAT: u32 = 0x7fff_f000; // saturated => effectively immortal (R: NAMEDMAX)
const PURPLE_CAP: usize = 1024;

const F_GRAY: u8 = 1 << 5;
const F_WHITE: u8 = 1 << 6;

// ---- layout -------------------------------------------------------------
// +0 type u8, +1 flags u8, +2 gp u16, +4 refcnt u32, +8 attrs *mut u8
// +16 gc_refs u32, +20 pad
// vectors   +24 len u32, +28 cap u32, payload +32
// cons      +24 car, +32 cdr, +40 tag
// env       +24 enclosure, +32 overflow chain, +40 count u32, +48..128 inline
// closure   +24 formals, +32 body, +40 env
// charsxp   +24 hash u32, +28 len u32, bytes +32 (NUL-terminated)
// symbol    +24 name, +32 hash u32 (immortal)

const OFF_GCREFS: usize = 16;
const OFF_FIELDS: usize = 24;

const ENV_SLOTS: usize = 5;

#[inline]
unsafe fn rd_ptr(p: *mut u8, off: usize) -> *mut u8 {
    *((p as *const *mut u8).add(off / 8))
}
#[inline]
unsafe fn wr_ptr(p: *mut u8, off: usize, v: *mut u8) {
    *((p as *mut *mut u8).add(off / 8)) = v;
}
#[inline]
unsafe fn rd_u32(p: *mut u8, off: usize) -> u32 {
    *((p as *const u32).add(off / 4))
}
#[inline]
unsafe fn wr_u32(p: *mut u8, off: usize, v: u32) {
    *((p as *mut u32).add(off / 4)) = v;
}

#[inline(always)]
unsafe fn hdr(p: *mut u8) -> u8 {
    *(p as *const u8)
}
#[inline(always)]
unsafe fn set_flags(p: *mut u8, f: u8) {
    *(p.add(1) as *mut u8) = f;
}
#[inline(always)]
unsafe fn get_flags(p: *mut u8) -> u8 {
    *(p.add(1) as *const u8)
}
#[inline(always)]
unsafe fn get_refcnt(p: *mut u8) -> u32 {
    *(p.add(4) as *const u32)
}
#[inline(always)]
unsafe fn set_refcnt(p: *mut u8, v: u32) {
    *(p.add(4) as *mut u32) = v;
}
#[inline(always)]
unsafe fn get_gc_refs(p: *mut u8) -> u32 {
    *(p.add(OFF_GCREFS) as *const u32)
}
#[inline(always)]
unsafe fn set_gc_refs(p: *mut u8, v: u32) {
    *(p.add(OFF_GCREFS) as *mut u32) = v;
}

// ---- global instance (single-threaded; drop glue must reach the manager)

struct Rt<T>(std::cell::UnsafeCell<*mut T>);
unsafe impl<T> Sync for Rt<T> {}
static MEM: Rt<RcMemFields> = Rt(std::cell::UnsafeCell::new(std::ptr::null_mut()));

#[inline(always)]
fn m() -> *mut RcMemFields {
    unsafe { *MEM.0.get() }
}

// ---- FxHash (public-domain multiply-xor-rotate)

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
}
impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut b = bytes;
        while b.len() >= 8 {
            self.add(u64::from_le_bytes(b[..8].try_into().unwrap()));
            b = &b[8..];
        }
        if b.len() >= 4 {
            self.add(u32::from_le_bytes(b[..4].try_into().unwrap()) as u64);
            b = &b[4..];
        }
        for &x in b {
            self.add(x as u64);
        }
    }
    #[inline]
    fn write_u64(&mut self, w: u64) {
        self.add(w);
    }
    #[inline]
    fn write_usize(&mut self, w: usize) {
        self.add(w as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}
type FxBuild = BuildHasherDefault<FxHasher>;

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(b);
    h.finish()
}

// ---- the manager

pub struct RcMemFields {
    heap: Heap,
    purple: Vec<*mut u8>,
    free_queue: Vec<*mut u8>,
    /// objects that died by count while still in the purple buffer: their
    /// memory must not be reused (or freed to the OS) before the next
    /// collection validates the buffer, or the validation would read freed
    /// (or reborn) objects
    dead_parked: Vec<*mut u8>,
    protect_stack: Vec<*mut u8>,
    nil_p: *mut u8,
    na_string: *mut u8,
    global_env: *mut u8,
    syms: HashMap<u64, Vec<*mut u8>, FxBuild>,
    chars: HashMap<u64, Vec<*mut u8>, FxBuild>,
    int_cache: Vec<*mut u8>, // value + 1024 for -1024..=8191
    pub stats: Stats,
    live_bytes: u64,
    live_objects: u64,
    last_pause_ns: u64,
    collecting: bool,
}

/// The manager. The fields live behind a `Box` so the process-global
/// pointer installed for the drop glue stays valid when the value moves.
pub struct RcMem(pub Box<RcMemFields>);

impl std::ops::Deref for RcMem {
    type Target = RcMemFields;
    #[inline]
    fn deref(&self) -> &RcMemFields {
        &self.0
    }
}
impl std::ops::DerefMut for RcMem {
    #[inline]
    fn deref_mut(&mut self) -> &mut RcMemFields {
        &mut self.0
    }
}

pub struct RcSexp(pub *mut u8);

/// One new counted reference to `p`, as a handle (the constructor itself
/// must not count: `RcSexp(p).clone()` returns an uncounted value because
/// the temporary's Drop cancels the clone's incref).
#[inline(always)]
fn counted(p: *mut u8) -> RcSexp {
    unsafe { incref(p) };
    RcSexp(p)
}

impl Clone for RcSexp {
    #[inline]
    fn clone(&self) -> Self {
        unsafe { incref(self.0) };
        RcSexp(self.0)
    }
}
impl Drop for RcSexp {
    #[inline]
    fn drop(&mut self) {
        unsafe { decref(self.0) };
    }
}

#[inline(always)]
unsafe fn incref(p: *mut u8) {
    let f = *(p.add(1) as *const u8);
    if f & flags::IMMORTAL != 0 {
        return;
    }
    let c = get_refcnt(p);
    if c < SAT {
        set_refcnt(p, c + 1);
    }
}

#[inline(always)]
unsafe fn decref(p: *mut u8) {
    let f = *(p.add(1) as *const u8);
    if f & flags::IMMORTAL != 0 {
        return;
    }
    let c = get_refcnt(p);
    if c >= SAT {
        return;
    }
    let c = c - 1;
    set_refcnt(p, c);
    let t = *(p as *const u8);
    if c == 0 {
        if f & flags::IN_COLLECTOR != 0 {
            return; // the cycle collector owns the free
        }
        let mm = m();
        if mm.is_null() {
            return;
        }
        (*mm).free_queue.push(p);
    } else if c == 1 && t == CHARSXP && f & flags::INTERNED != 0 {
        // only the intern table's own reference is left
        let mm = m();
        if mm.is_null() {
            return;
        }
        (*mm).chars_table_release(p);
    } else if is_container(t) && f & (flags::BUFFERED | flags::IN_COLLECTOR) == 0 {
        let mm = m();
        if mm.is_null() {
            return;
        }
        (*mm).maybe_purple(p);
    }
}

/// Enumerate every child pointer (attrs + fields + payload slots).
/// `f` must not free `p`.
unsafe fn for_each_child<F: FnMut(*mut u8)>(p: *mut u8, f: &mut F) {
    let h = hdr(p);
    let a = rd_ptr(p, 8);
    if !a.is_null() {
        f(a);
    }
    match h {
        LISTSXP | LANGSXP => {
            f(rd_ptr(p, OFF_FIELDS));
            f(rd_ptr(p, OFF_FIELDS + 8));
            f(rd_ptr(p, OFF_FIELDS + 16));
        }
        ENVSXP => {
            f(rd_ptr(p, OFF_FIELDS));
            f(rd_ptr(p, OFF_FIELDS + 8));
            let n = rd_u32(p, OFF_FIELDS + 16);
            for i in 0..n as usize {
                f(rd_ptr(p, OFF_FIELDS + 24 + i * 16 + 8));
            }
        }
        CLOSXP => {
            f(rd_ptr(p, OFF_FIELDS));
            f(rd_ptr(p, OFF_FIELDS + 8));
            f(rd_ptr(p, OFF_FIELDS + 16));
        }
        STRSXP | VECSXP => {
            let n = rd_u32(p, OFF_FIELDS);
            for i in 0..n as usize {
                let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                if !c.is_null() {
                    f(c);
                }
            }
        }
        SYMSXP => {
            f(rd_ptr(p, OFF_FIELDS));
        }
        _ => {}
    }
}

/// Can this object be part of a cycle? (container, or leaf with attributes)
#[inline]
unsafe fn rc_relevant(p: *mut u8) -> bool {
    let h = hdr(p);
    is_container(h) || !rd_ptr(p, 8).is_null()
}

fn elem_size(t: u8) -> usize {
    match t {
        REALSXP => 8,
        INTSXP | LGLSXP => 4,
        _ => 8, // STRSXP/VECSXP slots are pointers
    }
}

fn object_size(p: *mut u8) -> usize {
    unsafe {
        let h = hdr(p);
        match h {
            LISTSXP | LANGSXP | CLOSXP => 48,
            ENVSXP => 128,
            SYMSXP => 40,
            CHARSXP => {
                let l = rd_u32(p, OFF_FIELDS + 4) as usize;
                32 + l.div_ceil(16) * 16 + 16 // must match char_alloc
            }
            REALSXP | INTSXP | LGLSXP | STRSXP | VECSXP => {
                let cap = rd_u32(p, OFF_FIELDS + 4) as usize;
                32 + cap * elem_size(h)
            }
            _ => 48,
        }
    }
}

impl RcMemFields {
    /// Build and fully initialize the manager's state; `RcMem::new`
    /// installs the global pointer once the box is at its final address.
    fn init() -> RcMemFields {
        RcMemFields {
            heap: Heap::new(),
            purple: Vec::with_capacity(PURPLE_CAP + 1),
            free_queue: Vec::new(),
            dead_parked: Vec::new(),
            protect_stack: Vec::new(),
            nil_p: std::ptr::null_mut(),
            na_string: std::ptr::null_mut(),
            global_env: std::ptr::null_mut(),
            syms: HashMap::default(),
            chars: HashMap::default(),
            int_cache: vec![std::ptr::null_mut(); 1024 + 8192],
            stats: Stats::default(),
            live_bytes: 0,
            live_objects: 0,
            last_pause_ns: 0,
            collecting: false,
        }
    }

    fn bootstrap(&mut self) {
        unsafe {
            self.nil_p = self.alloc_object(NILSXP, 48);
            set_flags(self.nil_p, flags::IMMORTAL);
            set_refcnt(self.nil_p, SAT);
            self.na_string = self.char_alloc(b"NA");
            set_flags(self.na_string, flags::IMMORTAL);
            set_refcnt(self.na_string, SAT);
            let nil = RcSexp(self.nil_p);
            self.global_env = self.env_alloc(&nil);
            for v in -1024i32..8192 {
                let p = self.int_alloc(v);
                set_flags(p, flags::IMMORTAL);
                set_refcnt(p, SAT);
                self.int_cache[(v + 1024) as usize] = p;
            }
        }
    }
    fn alloc_object(&mut self, type_: u8, size: usize) -> *mut u8 {
        if !self.free_queue.is_empty() {
            // no cycle collection mid-drain: a cascade that trips the
            // purple threshold here would free objects the outer drain
            // has already popped (use-after-free)
            self.collecting = true;
            self.drain_frees();
            self.collecting = false;
        }
        let p = self.heap.alloc(size);
        unsafe {
            *(p as *mut u8) = type_;
            *(p.add(1) as *mut u8) = 0;
            *(p.add(2) as *mut u16) = 0;
            set_refcnt(p, 1); // the handle we return
            wr_ptr(p, 8, self.nil_p);
            set_gc_refs(p, 0);
        }
        self.stats.allocs += 1;
        self.stats.bytes_allocated += size as u64;
        self.live_bytes += size as u64;
        self.live_objects += 1;
        p
    }

    // -- kind constructors --------------------------------------------------

    fn vec_alloc(&mut self, type_: u8, len: usize, cap: usize) -> *mut u8 {
        let size = 32 + cap * elem_size(type_);
        let p = self.alloc_object(type_, size);
        unsafe {
            wr_u32(p, OFF_FIELDS, len as u32);
            wr_u32(p, OFF_FIELDS + 4, cap as u32);
            match type_ {
                // fill the whole capacity: `set_generic_elt`/`vector_push`
                // read-and-decref the old slot, which must be a valid
                // immortal pointer, never uninitialized memory
                STRSXP => {
                    for i in 0..cap {
                        wr_ptr(p, OFF_FIELDS + 8 + i * 8, self.na_string);
                    }
                }
                VECSXP => {
                    for i in 0..cap {
                        wr_ptr(p, OFF_FIELDS + 8 + i * 8, self.nil_p);
                    }
                }
                _ => {}
            }
        }
        p
    }

    fn int_alloc(&mut self, v: i32) -> *mut u8 {
        let p = self.vec_alloc(INTSXP, 1, 1);
        unsafe { *((p as *mut i32).add(OFF_FIELDS / 4 + 2)) = v } // payload at +32
        p
    }

    fn char_alloc(&mut self, bytes: &[u8]) -> *mut u8 {
        let size = 32 + bytes.len().div_ceil(16) * 16 + 16;
        let p = self.alloc_object(CHARSXP, size);
        unsafe {
            wr_u32(p, OFF_FIELDS, hash_bytes(bytes) as u32);
            wr_u32(p, OFF_FIELDS + 4, bytes.len() as u32);
            let dst = p.add(OFF_FIELDS + 8);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            *dst.add(bytes.len()) = 0;
        }
        p
    }

    fn env_alloc(&mut self, parent: &RcSexp) -> *mut u8 {
        let p = self.alloc_object(ENVSXP, 128);
        unsafe {
            wr_ptr(p, OFF_FIELDS, parent.0);
            wr_ptr(p, OFF_FIELDS + 8, self.nil_p);
            wr_u32(p, OFF_FIELDS + 16, 0);
            for i in 0..ENV_SLOTS {
                wr_ptr(p, OFF_FIELDS + 24 + i * 16, self.nil_p);
                wr_ptr(p, OFF_FIELDS + 24 + i * 16 + 8, self.nil_p);
            }
            incref(parent.0);
        }
        p
    }

    fn cons_alloc(&mut self, car: &RcSexp, cdr: &RcSexp, tag: &RcSexp, lang: bool) -> RcSexp {
        let p = self.alloc_object(if lang { LANGSXP } else { LISTSXP }, 48);
        unsafe {
            wr_ptr(p, OFF_FIELDS, car.0);
            wr_ptr(p, OFF_FIELDS + 8, cdr.0);
            wr_ptr(p, OFF_FIELDS + 16, tag.0);
            incref(car.0);
            incref(cdr.0);
            incref(tag.0);
        }
        RcSexp(p)
    }

    // -- environment frames -------------------------------------------------

    unsafe fn env_find(&self, env: *mut u8, sym: *mut u8) -> Option<*mut u8> {
        let n = rd_u32(env, OFF_FIELDS + 16);
        for i in 0..n as usize {
            if rd_ptr(env, OFF_FIELDS + 24 + i * 16) == sym {
                return Some(rd_ptr(env, OFF_FIELDS + 24 + i * 16 + 8));
            }
        }
        let mut cell = rd_ptr(env, OFF_FIELDS + 8);
        while cell != self.nil_p {
            if rd_ptr(cell, OFF_FIELDS + 16) == sym {
                return Some(rd_ptr(cell, OFF_FIELDS));
            }
            cell = rd_ptr(cell, OFF_FIELDS + 8);
        }
        None
    }

    // -- free / cycle machinery ---------------------------------------------

    fn maybe_purple(&mut self, p: *mut u8) {
        if !self.collecting && self.purple.len() >= PURPLE_CAP {
            self.collect_cycles();
        }
        unsafe {
            let f = get_flags(p);
            if f & flags::BUFFERED == 0 && get_refcnt(p) > 0 {
                set_flags(p, f | flags::BUFFERED);
                self.purple.push(p);
            }
        }
    }

    fn free_object(&mut self, p: *mut u8) {
        unsafe {
            set_refcnt(p, 0);
        }
        let size = object_size(p);
        self.stats.frees += 1;
        self.stats.live_bytes = self.live_bytes - size as u64;
        self.stats.live_objects = self.live_objects - 1;
        self.live_bytes -= size as u64;
        self.live_objects -= 1;
        if size > MAX_SMALL {
            self.heap.free_large(p);
        } else {
            self.heap.free_small(p);
        }
    }

    fn drain_frees(&mut self) {
        while let Some(p) = self.free_queue.pop() {
            unsafe {
                let f = get_flags(p);
                if f & flags::IN_COLLECTOR != 0 {
                    set_flags(p, f & !flags::IN_COLLECTOR);
                    self.free_object(p);
                    continue;
                }
                let attrs = rd_ptr(p, 8);
                if !attrs.is_null() {
                    decref(attrs);
                }
                let mut sink = |c: *mut u8| decref(c);
                for_each_child(p, &mut sink);
                if f & flags::BUFFERED != 0 {
                    // dead but still referenced by the purple buffer: park
                    // it (children released above) until the collection
                    // that consumes the buffer has validated it
                    self.dead_parked.push(p);
                    continue;
                }
            }
            self.free_object(p);
        }
    }

    /// Free the parked objects after a collection has validated (and
    /// discarded) the stale purple entries pointing at them.
    fn free_parked(&mut self) {
        while let Some(p) = self.dead_parked.pop() {
            unsafe {
                let f = get_flags(p);
                set_flags(p, f & !flags::BUFFERED);
            }
            self.free_object(p);
        }
    }

    fn chars_table_release(&mut self, p: *mut u8) {
        let hash = unsafe { rd_u32(p, OFF_FIELDS) } as u64;
        if let Some(bucket) = self.chars.get_mut(&hash) {
            bucket.retain(|&q| q != p);
            if bucket.is_empty() {
                self.chars.remove(&hash);
            }
        }
        unsafe { decref(p) }; // the table's reference
    }

    /// Bacon–Rajan synchronous cycle collection over the purple roots.
    fn collect_cycles(&mut self) {
        if self.collecting {
            return;
        }
        self.collecting = true;
        let t0 = Instant::now();
        self.drain_frees();
        let roots: Vec<*mut u8> = std::mem::take(&mut self.purple);
        let mut live_roots: Vec<*mut u8> = Vec::new();
        for &p in &roots {
            unsafe {
                if get_refcnt(p) == 0 || get_refcnt(p) >= SAT {
                    continue; // freed (stale entry) or effectively immortal
                }
                let f = get_flags(p);
                set_flags(p, f & !flags::BUFFERED);
            }
            live_roots.push(p);
        }

        // Phase 1: the purple-reachable subgraph; snapshot refcounts.
        let mut set: Vec<*mut u8> = Vec::new();
        let mut dfs: Vec<*mut u8> = live_roots.clone();
        while let Some(p) = dfs.pop() {
            unsafe {
                let f = get_flags(p);
                if f & F_GRAY != 0 {
                    continue;
                }
                set_flags(p, f | F_GRAY);
                set_gc_refs(p, get_refcnt(p));
                set.push(p);
                let mut push = |c: *mut u8| {
                    if rc_relevant(c) {
                        dfs.push(c);
                    }
                };
                for_each_child(p, &mut push);
            }
        }

        // Phase 2: subtract the internal edges.
        for &p in &set {
            unsafe {
                let mut sub = |c: *mut u8| {
                    if get_flags(c) & F_GRAY != 0 {
                        set_gc_refs(c, get_gc_refs(c) - 1);
                    }
                };
                for_each_child(p, &mut sub);
            }
        }

        // Phase 3: scan. Anchored objects (positive snapshot) paint their
        // reachable set black; the rest turn white.
        let mut black: Vec<*mut u8> = Vec::new();
        let mut scan: Vec<*mut u8> = live_roots.clone();
        while let Some(p) = scan.pop() {
            unsafe {
                let f = get_flags(p);
                if f & F_GRAY == 0 {
                    continue;
                }
                if get_gc_refs(p) > 0 {
                    black.push(p);
                    while let Some(q) = black.pop() {
                        let fq = get_flags(q);
                        if fq & (F_GRAY | F_WHITE) == 0 {
                            continue;
                        }
                        set_flags(q, fq & !(F_GRAY | F_WHITE));
                        let mut paint = |c: *mut u8| {
                            if rc_relevant(c) {
                                black.push(c);
                            }
                        };
                        for_each_child(q, &mut paint);
                    }
                } else {
                    set_flags(p, (f & !F_GRAY) | F_WHITE);
                    let mut push = |c: *mut u8| {
                        if rc_relevant(c) {
                            scan.push(c);
                        }
                    };
                    for_each_child(p, &mut push);
                }
            }
        }

        // Phase 4: collect the whites; release their child references by
        // ordinary decrements so survivors keep exact counts.
        let mut whites: Vec<*mut u8> = Vec::new();
        let mut wstack: Vec<*mut u8> = live_roots.clone();
        while let Some(p) = wstack.pop() {
            unsafe {
                let f = get_flags(p);
                if f & F_WHITE == 0 {
                    continue;
                }
                set_flags(p, (f & !(F_WHITE | F_GRAY)) | flags::IN_COLLECTOR);
                whites.push(p);
                let mut push = |c: *mut u8| {
                    if rc_relevant(c) {
                        wstack.push(c);
                    }
                };
                for_each_child(p, &mut push);
            }
        }
        for &p in &whites {
            unsafe {
                let mut dec = |c: *mut u8| decref(c);
                for_each_child(p, &mut dec);
                let a = rd_ptr(p, 8);
                if !a.is_null() {
                    decref(a);
                }
            }
        }
        for &p in &whites {
            self.free_queue.push(p);
        }
        self.drain_frees();
        // the purple buffer has been consumed and validated above: the
        // memory of buffered objects that died by count can flow again
        self.free_parked();

        let dt = t0.elapsed().as_nanos() as u64;
        self.stats.collections += 1;
        self.stats.collect_total_ns += dt;
        self.stats.collect_max_pause_ns = self.stats.collect_max_pause_ns.max(dt);
        self.last_pause_ns = dt;
        self.collecting = false;
    }
}

impl RcMem {
    pub fn new() -> RcMem {
        let mut mem = RcMem(Box::new(RcMemFields::init()));
        unsafe {
            *MEM.0.get() = &mut *mem.0;
        }
        mem.bootstrap();
        mem
    }
}

impl Default for RcMem {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RcMem {
    fn drop(&mut self) {
        unsafe {
            *MEM.0.get() = std::ptr::null_mut();
        }
    }
}

impl Mem for RcMem {
    type Handle = RcSexp;

    fn create() -> Self {
        RcMem::new()
    }
    fn nil(&self) -> RcSexp {
        RcSexp(self.nil_p)
    }
    #[inline]
    fn retain(&self, h: &RcSexp) -> RcSexp {
        h.clone()
    }
    #[inline]
    fn discard(&mut self, h: RcSexp) {
        drop(h);
    }

    fn alloc_real(&mut self, len: usize) -> RcSexp {
        RcSexp(self.vec_alloc(REALSXP, len, len))
    }
    fn alloc_int(&mut self, len: usize) -> RcSexp {
        RcSexp(self.vec_alloc(INTSXP, len, len))
    }
    fn alloc_generic(&mut self, len: usize) -> RcSexp {
        // lists grow by appending: give headroom like R's vector utils do
        let cap = len.max(4).next_power_of_two();
        RcSexp(self.vec_alloc(VECSXP, len, cap))
    }
    fn alloc_string(&mut self, len: usize) -> RcSexp {
        RcSexp(self.vec_alloc(STRSXP, len, len))
    }

    fn cons(&mut self, car: &RcSexp, cdr: &RcSexp, tag: &RcSexp, lang: bool) -> RcSexp {
        self.cons_alloc(car, cdr, tag, lang)
    }

    fn env_new(&mut self, parent: &RcSexp) -> RcSexp {
        RcSexp(self.env_alloc(parent))
    }

    fn closure_new(&mut self, formals: &RcSexp, body: &RcSexp, env: &RcSexp) -> RcSexp {
        let p = self.alloc_object(CLOSXP, 48);
        unsafe {
            wr_ptr(p, OFF_FIELDS, formals.0);
            wr_ptr(p, OFF_FIELDS + 8, body.0);
            wr_ptr(p, OFF_FIELDS + 16, env.0);
            incref(formals.0);
            incref(body.0);
            incref(env.0);
        }
        RcSexp(p)
    }

    fn char_new(&mut self, bytes: &[u8]) -> RcSexp {
        RcSexp(self.char_alloc(bytes))
    }

    fn char_intern(&mut self, bytes: &[u8]) -> RcSexp {
        // key by the hash the object caches (u32): `chars_table_release`
        // looks the entry up with the cached value, so both must agree
        let hash = hash_bytes(bytes) as u32 as u64;
        if let Some(bucket) = self.chars.get(&hash) {
            for &q in bucket {
                unsafe {
                    if rd_u32(q, OFF_FIELDS + 4) as usize == bytes.len() {
                        let qb = q.add(OFF_FIELDS + 8);
                        if std::slice::from_raw_parts(qb, bytes.len()) == bytes {
                            return counted(q);
                        }
                    }
                }
            }
        }
        let p = self.char_alloc(bytes);
        unsafe {
            set_flags(p, get_flags(p) | flags::INTERNED);
            incref(p); // the intern table's own reference
        }
        self.chars.entry(hash).or_default().push(p);
        RcSexp(p)
    }

    fn symbol_intern(&mut self, name: &[u8]) -> RcSexp {
        let hash = hash_bytes(name) as u64;
        if let Some(bucket) = self.syms.get(&hash) {
            for &q in bucket {
                unsafe {
                    let nm = rd_ptr(q, OFF_FIELDS);
                    if rd_u32(nm, OFF_FIELDS + 4) as usize == name.len() {
                        let nb = nm.add(OFF_FIELDS + 8);
                        if std::slice::from_raw_parts(nb, name.len()) == name {
                            return RcSexp(q);
                        }
                    }
                }
            }
        }
        let nm = self.char_alloc(name);
        let p = self.alloc_object(SYMSXP, 40);
        unsafe {
            wr_ptr(p, OFF_FIELDS, nm);
            wr_u32(p, OFF_FIELDS + 8, hash as u32);
            set_flags(p, flags::IMMORTAL);
            set_refcnt(p, SAT);
        }
        self.syms.entry(hash).or_default().push(p);
        RcSexp(p)
    }

    fn int_scalar(&mut self, v: i32) -> RcSexp {
        if (-1024..8192).contains(&v) {
            return RcSexp(self.int_cache[(v + 1024) as usize]);
        }
        RcSexp(self.int_alloc(v))
    }

    fn real_scalar(&mut self, v: f64) -> RcSexp {
        let p = self.vec_alloc(REALSXP, 1, 1);
        unsafe { *((p as *mut f64).add(4)) = v } // payload at +32 = 4 * f64
        RcSexp(p)
    }

    // ---- access ---------------------------------------------------------

    fn sexp_type(&self, v: &RcSexp) -> u8 {
        unsafe { *(v.0 as *const u8) }
    }
    fn real_data(&self, v: &RcSexp) -> *mut f64 {
        unsafe { v.0.add(OFF_FIELDS + 8) as *mut f64 }
    }
    fn real_len(&self, v: &RcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }
    fn int_data(&self, v: &RcSexp) -> *mut i32 {
        unsafe { v.0.add(OFF_FIELDS + 8) as *mut i32 }
    }
    fn int_len(&self, v: &RcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }
    fn scalar_int_value(&self, v: &RcSexp) -> i32 {
        unsafe { *(v.0.add(OFF_FIELDS + 8) as *const i32) }
    }

    fn generic_elt(&self, v: &RcSexp, i: usize) -> RcSexp {
        counted(unsafe { rd_ptr(v.0, OFF_FIELDS + 8 + i * 8) })
    }
    fn set_generic_elt(&mut self, v: &RcSexp, i: usize, val: &RcSexp) {
        unsafe {
            let slot = v.0.add(OFF_FIELDS + 8 + i * 8);
            let old = *(slot as *mut *mut u8);
            incref(val.0);
            *(slot as *mut *mut u8) = val.0;
            if old != self.nil_p {
                decref(old);
            }
        }
    }
    fn generic_len(&self, v: &RcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }

    fn string_elt(&self, v: &RcSexp, i: usize) -> RcSexp {
        self.generic_elt(v, i)
    }
    fn set_string_elt(&mut self, v: &RcSexp, i: usize, val: &RcSexp) {
        self.set_generic_elt(v, i, val)
    }
    fn string_len(&self, v: &RcSexp) -> usize {
        self.generic_len(v)
    }
    fn char_bytes(&self, c: &RcSexp) -> &[u8] {
        unsafe {
            let l = rd_u32(c.0, OFF_FIELDS + 4) as usize;
            std::slice::from_raw_parts(c.0.add(OFF_FIELDS + 8), l)
        }
    }

    fn car(&self, v: &RcSexp) -> RcSexp {
        counted(unsafe { rd_ptr(v.0, OFF_FIELDS) })
    }
    fn cdr(&self, v: &RcSexp) -> RcSexp {
        counted(unsafe { rd_ptr(v.0, OFF_FIELDS + 8) })
    }
    fn tag(&self, v: &RcSexp) -> RcSexp {
        counted(unsafe { rd_ptr(v.0, OFF_FIELDS + 16) })
    }
    fn set_car(&mut self, v: &RcSexp, val: &RcSexp) {
        unsafe {
            let old = rd_ptr(v.0, OFF_FIELDS);
            incref(val.0);
            wr_ptr(v.0, OFF_FIELDS, val.0);
            decref(old);
        }
    }

    fn env_define(&mut self, env: &RcSexp, sym: &RcSexp, val: &RcSexp) {
        unsafe {
            let e = env.0;
            let n = rd_u32(e, OFF_FIELDS + 16);
            if n < ENV_SLOTS as u32 {
                wr_ptr(e, OFF_FIELDS + 24 + n as usize * 16, sym.0);
                wr_ptr(e, OFF_FIELDS + 24 + n as usize * 16 + 8, val.0);
                wr_u32(e, OFF_FIELDS + 16, n + 1);
                incref(val.0);
                return;
            }
            // append to the overflow chain; the chain owns the cell
            let nilh = RcSexp(self.nil_p);
            let cell = self.cons_alloc(val, &nilh, sym, false);
            let cp = cell.0;
            std::mem::forget(cell);
            let mut tail = rd_ptr(e, OFF_FIELDS + 8);
            if tail == self.nil_p {
                wr_ptr(e, OFF_FIELDS + 8, cp);
            } else {
                while rd_ptr(tail, OFF_FIELDS + 8) != self.nil_p {
                    tail = rd_ptr(tail, OFF_FIELDS + 8);
                }
                wr_ptr(tail, OFF_FIELDS + 8, cp);
            }
        }
    }

    fn env_set(&mut self, env: &RcSexp, sym: &RcSexp, val: &RcSexp) {
        unsafe {
            let e = env.0;
            let n = rd_u32(e, OFF_FIELDS + 16);
            for i in 0..n as usize {
                if rd_ptr(e, OFF_FIELDS + 24 + i * 16) == sym.0 {
                    let slot = e.add(OFF_FIELDS + 24 + i * 16 + 8);
                    let old = *(slot as *mut *mut u8);
                    incref(val.0);
                    *(slot as *mut *mut u8) = val.0;
                    decref(old);
                    return;
                }
            }
            let mut cell = rd_ptr(e, OFF_FIELDS + 8);
            while cell != self.nil_p {
                if rd_ptr(cell, OFF_FIELDS + 16) == sym.0 {
                    let old = rd_ptr(cell, OFF_FIELDS);
                    incref(val.0);
                    wr_ptr(cell, OFF_FIELDS, val.0);
                    decref(old);
                    return;
                }
                cell = rd_ptr(cell, OFF_FIELDS + 8);
            }
        }
    }

    fn env_take(&mut self, env: &RcSexp, sym: &RcSexp) -> Option<RcSexp> {
        unsafe {
            let e = env.0;
            let n = rd_u32(e, OFF_FIELDS + 16);
            for i in 0..n as usize {
                if rd_ptr(e, OFF_FIELDS + 24 + i * 16) == sym.0 {
                    let val = rd_ptr(e, OFF_FIELDS + 24 + i * 16 + 8);
                    let last = n as usize - 1;
                    let lsym = rd_ptr(e, OFF_FIELDS + 24 + last * 16);
                    let lval = rd_ptr(e, OFF_FIELDS + 24 + last * 16 + 8);
                    wr_ptr(e, OFF_FIELDS + 24 + i * 16, lsym);
                    wr_ptr(e, OFF_FIELDS + 24 + i * 16 + 8, lval);
                    wr_u32(e, OFF_FIELDS + 16, n - 1);
                    return Some(RcSexp(val)); // the binding's reference transfers
                }
            }
            let mut prev: *mut u8 = std::ptr::null_mut();
            let mut cell = rd_ptr(e, OFF_FIELDS + 8);
            while cell != self.nil_p {
                if rd_ptr(cell, OFF_FIELDS + 16) == sym.0 {
                    let next = rd_ptr(cell, OFF_FIELDS + 8);
                    if prev.is_null() {
                        wr_ptr(e, OFF_FIELDS + 8, next);
                    } else {
                        wr_ptr(prev, OFF_FIELDS + 8, next);
                    }
                    let val = rd_ptr(cell, OFF_FIELDS);
                    self.free_object(cell); // car/cdr/tag references transferred
                    return Some(RcSexp(val));
                }
                prev = cell;
                cell = rd_ptr(cell, OFF_FIELDS + 8);
            }
            None
        }
    }

    fn env_lookup(&mut self, env: &RcSexp, sym: &RcSexp) -> Option<RcSexp> {
        unsafe { self.env_find(env.0, sym.0).map(counted) }
    }

    fn global_env(&self) -> RcSexp {
        // counted: the manager keeps its own reference to the root; the
        // temporary handles workloads pass around must not free it
        counted(self.global_env)
    }

    fn attrs_set(&mut self, obj: &RcSexp, attrs: &RcSexp) {
        unsafe {
            let old = rd_ptr(obj.0, 8);
            incref(attrs.0);
            wr_ptr(obj.0, 8, attrs.0);
            if old != self.nil_p {
                decref(old);
            }
        }
    }
    fn attrs_get(&self, obj: &RcSexp) -> RcSexp {
        counted(unsafe { rd_ptr(obj.0, 8) })
    }

    fn external_refs(&self, h: &RcSexp) -> u32 {
        let c = unsafe { get_refcnt(h.0) };
        if c >= SAT { 0 } else { c - 1 }
    }

    fn duplicate(&mut self, h: &RcSexp) -> RcSexp {
        unsafe {
            let p = h.0;
            let t = *(p as *const u8);
            let len = rd_u32(p, OFF_FIELDS) as usize;
            let attrs = rd_ptr(p, 8);
            match t {
                REALSXP | INTSXP | LGLSXP => {
                    let q = self.vec_alloc(t, len, len);
                    std::ptr::copy_nonoverlapping(
                        p.add(OFF_FIELDS + 8) as *const u8,
                        q.add(OFF_FIELDS + 8) as *mut u8,
                        len * elem_size(t),
                    );
                    if attrs != self.nil_p {
                        incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    RcSexp(q)
                }
                STRSXP | VECSXP => {
                    let q = self.vec_alloc(t, len, len);
                    for i in 0..len {
                        let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                        incref(c);
                        wr_ptr(q, OFF_FIELDS + 8 + i * 8, c);
                    }
                    if attrs != self.nil_p {
                        incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    RcSexp(q)
                }
                ENVSXP => h.clone(), // environments are reference objects
                CHARSXP | SYMSXP | NILSXP => h.clone(),
                _ => {
                    // cons/closure: shallow structural copy with counted refs
                    let q = self.alloc_object(t, object_size(p));
                    for off in [OFF_FIELDS, OFF_FIELDS + 8, OFF_FIELDS + 16] {
                        let c = rd_ptr(p, off);
                        incref(c);
                        wr_ptr(q, off, c);
                    }
                    if attrs != self.nil_p {
                        incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    RcSexp(q)
                }
            }
        }
    }

    fn vector_push(&mut self, v: &RcSexp, val: &RcSexp) -> RcSexp {
        unsafe {
            let p = v.0;
            let len = rd_u32(p, OFF_FIELDS) as usize;
            let cap = rd_u32(p, OFF_FIELDS + 4) as usize;
            if len < cap {
                let slot = p.add(OFF_FIELDS + 8 + len * 8);
                let old = *(slot as *mut *mut u8);
                incref(val.0);
                *(slot as *mut *mut u8) = val.0;
                if old != self.nil_p && old != self.na_string {
                    decref(old);
                }
                wr_u32(p, OFF_FIELDS, (len + 1) as u32);
                return v.clone();
            }
            let new_cap = (cap * 2).max(4);
            let q = self.vec_alloc(VECSXP, len + 1, new_cap);
            for i in 0..len {
                let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                incref(c);
                wr_ptr(q, OFF_FIELDS + 8 + i * 8, c);
            }
            incref(val.0);
            wr_ptr(q, OFF_FIELDS + 8 + len * 8, val.0);
            let attrs = rd_ptr(p, 8);
            if attrs != self.nil_p {
                incref(attrs);
                wr_ptr(q, 8, attrs);
            }
            RcSexp(q)
        }
    }

    fn vector_capacity(&self, v: &RcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS + 4) as usize }
    }

    fn collect(&mut self) -> u64 {
        self.collect_cycles();
        self.last_pause_ns
    }

    fn stats(&self) -> Stats {
        let mut s = self.stats.clone();
        s.live_bytes = self.live_bytes;
        s.live_objects = self.live_objects;
        s.protected_now = self.protect_stack.len();
        s
    }

    fn protect(&mut self, h: &RcSexp) -> RcSexp {
        unsafe { incref(h.0) }; // the protection stack's reference
        self.protect_stack.push(h.0);
        counted(h.0) // the returned handle owns its own count
    }
    fn unprotect(&mut self, n: usize) {
        for _ in 0..n {
            if let Some(p) = self.protect_stack.pop() {
                unsafe { decref(p) };
            }
        }
    }

    fn heap_reserved(&self) -> usize {
        self.heap.reserved
    }
}

// A helper that forgets a handle without dropping the reference.
#[allow(dead_code)]
fn cell_no_drop(p: *mut u8) -> RcSexp {
    RcSexp(p) // caller must forget to drop
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RcMem installs a process-global pointer for its drop glue, so tests
    /// (which the harness runs on parallel threads) must not overlap: hold
    /// the guard for the whole test.
    fn mk() -> (std::sync::MutexGuard<'static, ()>, RcMem) {
        static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // the guard must drop AFTER the manager: RcMem::drop resets the
        // process-global pointer, and the next test installs its own only
        // under the lock
        (g, RcMem::new())
    }

    #[test]
    fn basic_counts() {
        let (_g, mut m) = mk();
        let v = m.alloc_real(10);
        assert_eq!(m.external_refs(&v), 0);
        let w = m.retain(&v);
        assert_eq!(m.external_refs(&v), 1);
        drop(w);
        assert_eq!(m.external_refs(&v), 0);
        drop(v);
        m.collect();
        assert_eq!(m.stats().live_objects, m.stats().allocs - m.stats().frees);
    }

    #[test]
    fn cycles_collected() {
        let (_g, mut m) = mk();
        let base = m.stats().live_objects;
        let n = match std::env::var("CYCLES_N") {
            Ok(v) => v.parse().unwrap(),
            Err(_) => 5000,
        };
        for _i in 0..n {
            let nil = m.nil();
            let e = m.env_new(&m.global_env());
            let c = m.closure_new(&nil, &nil, &e);
            let s = m.symbol_intern(b"g");
            m.env_define(&e, &s, &c);
            m.discard(c);
            m.discard(e);
        }
        m.collect();
        let live = m.stats().live_objects - base;
        assert!(live < 10, "cycle leak: {live} live objects remain");
    }

    #[test]
    fn interned_chars_share() {
        let (_g, mut m) = mk();
        let a = m.char_intern(b"hello");
        let b = m.char_intern(b"hello");
        assert_eq!(a.0, b.0); // identical byte strings share one object
        let base = m.stats().live_objects;
        drop(b);
        drop(a);
        m.collect(); // drains the free queue
        assert_eq!(m.stats().live_objects, base - 1, "the char and its table entry must die");
        // while a handle lives, the interned char stays alive
        let c = m.char_intern(b"hello");
        let d = m.char_intern(b"hello");
        m.collect();
        assert_eq!(m.char_bytes(&c), b"hello");
        drop(c);
        drop(d);
    }

    #[test]
    fn env_take_transfers() {
        let (_g, mut m) = mk();
        let s = m.symbol_intern(b"x");
        let v = m.real_scalar(3.5);
        m.env_define(&m.global_env(), &s, &v);
        assert_eq!(m.external_refs(&v), 1);
        drop(v);
        let zero = m.int_scalar(0);
        assert_eq!(m.scalar_int_value(&zero), 0);
        let w = m.env_take(&m.global_env(), &s).unwrap();
        drop(w);
        assert!(m.env_lookup(&m.global_env(), &s).is_none());
    }
}
