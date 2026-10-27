//! Variant (b): a non-moving mark-region heap (Immix-shaped: 64 KiB block
//! regions, size classes, free lists rebuilt by the sweeper) with
//! conservative scanning of the Rust stack and registers, interior
//! pointers resolved to object starts, and reference counts kept **only**
//! for copy-on-modify (heap-to-heap references and bindings, like GNU R's
//! REFCNT since 4.0 — `MAYBE_SHARED(x)` needs exact binding counts, stack
//! locals are not counted because the collector, not the count, is
//! responsible for liveness).
//!
//! Rust locals hold plain `Copy` pointers: no increment on copy, no work
//! on drop. A precise pointer-protection stack (`PROTECT`/`UNPROTECT`)
//! serves C code; conservative scanning of the whole machine stack
//! (registers spilled by a small assembly shim, x86_64) catches the rest.
//!
//! Layout: 16-byte header (type, flags, gp, copy-on-modify count,
//! attributes), kind fields at +16 — cons 40, env 120, closure 40,
//! vectors `24 + cap * elt`, CHARSXP `24 + bytes`, all 16-byte granules.
//! Objects never move, so `REAL(x)` stays valid across collections.

#![allow(unsafe_op_in_unsafe_fn)]

use mm_core::sexp::*;
use mm_core::{flags, Mem, Stats};
use mm_heap::{Block, Heap, SIZES, BLOCK, CLASS_NONE, OWNER_EMPTY, OWNER_LIST, OWNER_NONE};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant;

const SAT: u32 = 0x7fff_f000;

/// Grow the heap at least this much between GCs. Overridable with
/// MM_GC_MIN (bytes) so tests can stress with tiny heaps.
#[inline]
fn gc_min() -> usize {
    static V: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("MM_GC_MIN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8 << 20)
    })
}

// ---- layout -------------------------------------------------------------
// +0 type u8, +1 flags u8, +2 gp u16, +4 com refcount u32, +8 attrs *mut u8
// vectors   +16 len u32, +20 cap u32, payload +24
// cons      +16 car, +24 cdr, +32 tag
// env       +16 enclosure, +24 overflow, +32 count u32, +40..120 inline[5]
// closure   +16 formals, +24 body, +32 env
// charsxp   +16 hash u32, +20 len u32, bytes +24
// symbol    +16 name, +24 hash u32 (immortal, name immortal)

const OFF_FIELDS: usize = 16;
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
unsafe fn get_flags(p: *mut u8) -> u8 {
    *(p.add(1) as *const u8)
}
#[inline(always)]
unsafe fn set_flags(p: *mut u8, f: u8) {
    *(p.add(1) as *mut u8) = f;
}
#[inline(always)]
unsafe fn get_com(p: *mut u8) -> u32 {
    *(p.add(4) as *const u32)
}

/// Copy-on-modify reference count: heap slots and bindings only.
#[inline(always)]
unsafe fn com_incref(p: *mut u8) {
    let f = get_flags(p);
    if f & flags::IMMORTAL == 0 {
        let c = get_com(p);
        if c < SAT {
            *(p.add(4) as *mut u32) = c + 1;
        }
    }
}
#[inline(always)]
unsafe fn com_decref(p: *mut u8) {
    let f = get_flags(p);
    if f & flags::IMMORTAL == 0 {
        let c = get_com(p);
        if c > 0 {
            *(p.add(4) as *mut u32) = c - 1;
        }
    }
}

// ---- register spill (x86_64): keep roots that live only in registers

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl mm_gc_spill",
    "mm_gc_spill:",
    "push rbp",
    "mov rbp, rsp",
    "push rax",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rax, rdi",   // rax = callback
    "mov rdi, rsi",   // rdi = ctx
    "mov rsi, rsp",   // rsi = sp bound (saved registers included)
    "call rax",
    "lea rsp, [rbp - 112]",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "pop rbp",
    "ret",
);

#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    fn mm_gc_spill(cb: unsafe extern "C" fn(ctx: *mut u8, sp: usize), ctx: *mut u8);
}

#[cfg(not(target_arch = "x86_64"))]
unsafe extern "C" fn mm_gc_spill(cb: unsafe extern "C" fn(ctx: *mut u8, sp: usize), ctx: *mut u8) {
    let sp = &ctx as *const _ as usize;
    cb(ctx, sp);
}

// ---- FxHash (same as mm-rc)

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

pub struct GcMem {
    heap: Heap,
    protect_stack: Vec<*mut u8>,
    nil_p: *mut u8,
    na_string: *mut u8,
    global_env: *mut u8,
    syms: HashMap<u64, Vec<*mut u8>, FxBuild>,
    chars: HashMap<u64, Vec<*mut u8>, FxBuild>, // weak: not a root
    int_cache: Vec<*mut u8>,
    mark_stack: Vec<*mut u8>,
    alloc_since_gc: usize,
    gc_threshold: usize,
    stack_top: usize,
    pub stats: Stats,
    live_bytes: u64,
    live_objects: u64,
    last_pause_ns: u64,
}

#[derive(Clone, Copy)]
pub struct GcSexp(pub *mut u8);

unsafe fn mark_obj(g: *mut GcMem, p: *mut u8) {
    if p.is_null() {
        return;
    }
    let f = get_flags(p);
    if f & (flags::MARK | flags::IMMORTAL) != 0 {
        return;
    }
    set_flags(p, f | flags::MARK);
    (*g).mark_stack.push(p);
}

unsafe fn trace(g: *mut GcMem) {
    let nil = (*g).nil_p;
    while let Some(p) = (*g).mark_stack.pop() {
        let a = rd_ptr(p, 8);
        if !a.is_null() {
            mark_obj(g, a);
        }
        let t = *(p as *const u8);
        match t {
            LISTSXP | LANGSXP | CLOSXP => {
                mark_obj(g, rd_ptr(p, OFF_FIELDS));
                mark_obj(g, rd_ptr(p, OFF_FIELDS + 8));
                mark_obj(g, rd_ptr(p, OFF_FIELDS + 16));
            }
            ENVSXP => {
                mark_obj(g, rd_ptr(p, OFF_FIELDS));
                mark_obj(g, rd_ptr(p, OFF_FIELDS + 8));
                let n = rd_u32(p, OFF_FIELDS + 16);
                for i in 0..n as usize {
                    mark_obj(g, rd_ptr(p, OFF_FIELDS + 24 + i * 16 + 8));
                }
            }
            STRSXP | VECSXP => {
                let n = rd_u32(p, OFF_FIELDS);
                for i in 0..n as usize {
                    let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                    if c != nil {
                        mark_obj(g, c);
                    }
                }
            }
            SYMSXP => mark_obj(g, rd_ptr(p, OFF_FIELDS)),
            _ => {}
        }
    }
}

pub static WATCH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C" fn scan_and_mark(ctx: *mut u8, sp: usize) {
    let g = ctx as *mut GcMem;
    let watch = WATCH.load(std::sync::atomic::Ordering::Relaxed);
    let top = (*g).stack_top;
    let heap = &raw mut (*g).heap;
    let min = (*heap).min_addr;
    let max = (*heap).max_addr;
    let mut w = (sp + 7) & !7usize;
    if watch != 0 {
        eprintln!("scan sp={sp:#x} top={top:#x} words={}", (top - sp) / 8);
    }
    while w < top {
        let v = *(w as *const usize);
        if v == watch && watch != 0 {
            eprintln!("WATCH found on stack at w={w:#x} v={v:#x}");
        }
        if v >= min && v < max {
            if let Some(p) = (*heap).resolve(v) {
                if watch != 0 && v == watch {
                    eprintln!("WATCH in heap range, resolved to {p:?}");
                }
                mark_obj(g, p);
            } else if watch != 0 && v == watch {
                eprintln!("WATCH in range but resolve FAILED v={v:#x}");
            }
        }
        w += 8;
    }
    trace(g);
}

impl GcMem {
    pub fn new() -> GcMem {
        let mut mem = GcMem {
            heap: Heap::new(),
            protect_stack: Vec::new(),
            nil_p: std::ptr::null_mut(),
            na_string: std::ptr::null_mut(),
            global_env: std::ptr::null_mut(),
            syms: HashMap::default(),
            chars: HashMap::default(),
            int_cache: vec![std::ptr::null_mut(); 1024 + 8192],
            mark_stack: Vec::new(),
            alloc_since_gc: 0,
            gc_threshold: gc_min(),
            stack_top: 0,
            stats: Stats::default(),
            live_bytes: 0,
            live_objects: 0,
            last_pause_ns: 0,
        };
        let sp = &mem as *const _ as usize;
        mem.stack_top = sp;
        unsafe {
            mem.nil_p = mem.alloc_object(NILSXP, 32);
            set_flags(mem.nil_p, flags::IMMORTAL);
            mem.na_string = mem.char_alloc(b"NA");
            set_flags(mem.na_string, flags::IMMORTAL);
            let nil = GcSexp(mem.nil_p);
            mem.global_env = mem.env_alloc(&nil);
            for v in -1024i32..8192 {
                let p = mem.int_alloc(v);
                set_flags(p, flags::IMMORTAL);
                mem.int_cache[(v + 1024) as usize] = p;
            }
        }
        mem
    }

    fn alloc_object(&mut self, type_: u8, size: usize) -> *mut u8 {
        if self.alloc_since_gc >= self.gc_threshold {
            self.collect();
        }
        let p = self.heap.alloc(size);
        unsafe {
            *(p as *mut u8) = type_;
            *(p.add(1) as *mut u8) = 0;
            *(p.add(2) as *mut u16) = 0;
            *(p.add(4) as *mut u32) = 0; // no heap references yet
            wr_ptr(p, 8, self.nil_p);
        }
        self.alloc_since_gc += size;
        self.stats.allocs += 1;
        self.stats.bytes_allocated += size as u64;
        self.live_bytes += size as u64;
        self.live_objects += 1;
        p
    }

    fn vec_alloc(&mut self, type_: u8, len: usize, cap: usize) -> *mut u8 {
        let esz = match type_ {
            REALSXP => 8,
            INTSXP | LGLSXP => 4,
            _ => 8,
        };
        let size = 24 + cap * esz;
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
        unsafe { *(p.add(OFF_FIELDS + 8) as *mut i32) = v }
        p
    }

    fn char_alloc(&mut self, bytes: &[u8]) -> *mut u8 {
        let size = 24 + bytes.len().div_ceil(16) * 16 + 16;
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

    fn env_alloc(&mut self, parent: &GcSexp) -> *mut u8 {
        let p = self.alloc_object(ENVSXP, 120);
        unsafe {
            wr_ptr(p, OFF_FIELDS, parent.0);
            wr_ptr(p, OFF_FIELDS + 8, self.nil_p);
            wr_u32(p, OFF_FIELDS + 16, 0);
            for i in 0..ENV_SLOTS {
                wr_ptr(p, OFF_FIELDS + 24 + i * 16, self.nil_p);
                wr_ptr(p, OFF_FIELDS + 24 + i * 16 + 8, self.nil_p);
            }
            com_incref(parent.0);
        }
        p
    }

    fn cons_alloc(&mut self, car: &GcSexp, cdr: &GcSexp, tag: &GcSexp, lang: bool) -> GcSexp {
        let p = self.alloc_object(if lang { LANGSXP } else { LISTSXP }, 40);
        unsafe {
            wr_ptr(p, OFF_FIELDS, car.0);
            wr_ptr(p, OFF_FIELDS + 8, cdr.0);
            wr_ptr(p, OFF_FIELDS + 16, tag.0);
            com_incref(car.0);
            com_incref(cdr.0);
            com_incref(tag.0);
        }
        GcSexp(p)
    }

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

    fn prune_chars(&mut self) {
        let mut pruned = 0u64;
        self.chars.retain(|_, bucket| {
            bucket.retain(|&q| {
                let live = unsafe { get_flags(q) } & (flags::MARK | flags::IMMORTAL) != 0;
                if !live {
                    pruned += 1;
                }
                live
            });
            !bucket.is_empty()
        });
        self.stats.chars_interned -= pruned.min(self.stats.chars_interned);
    }

    /// Mark-region sweep: free lines of dead objects back to their block's
    /// free list, recycle fully-empty blocks, release dead large chunks.
    /// Dead objects' child references are subtracted from the survivors'
    /// copy-on-modify counts (decrementing a dead child is harmless: it is
    /// about to be freed anyway).
    fn sweep(&mut self) {
        let mut live_bytes = 0u64;
        let mut live_objects = 0u64;
        let mut freed = 0u64;
        let mut to_list: Vec<(usize, *mut Block)> = Vec::new();
        let mut to_pool: Vec<*mut Block> = Vec::new();
        let mut dead_chunks: Vec<usize> = Vec::new();
        // chunks whose every claimed block died this sweep (candidates to
        // release together with their empty-pool blocks)
        let mut dead_small_chunks: Vec<(usize, usize, usize)> = Vec::new();

        let chunks: Vec<mm_heap::Chunk> = self.heap.chunks.clone();
        for (idx, c) in chunks.iter().enumerate() {
            if c.large {
                let obj = (c.base + mm_heap::DATA_OFF) as *mut u8;
                let f = unsafe { get_flags(obj) };
                if f & (flags::MARK | flags::IMMORTAL) != 0 {
                    if f & flags::MARK != 0 {
                        unsafe { set_flags(obj, f & !flags::MARK) };
                    }
                    live_bytes += (c.end - c.base) as u64;
                    live_objects += 1;
                } else {
                    dead_chunks.push(idx);
                }
                continue;
            }
            let mut chunk_dead = true; // every claimed block empty after sweep?
            let mut chunk_touched = false;
            for bi in 0..c.claimed as usize {
                let b = (c.base + bi * BLOCK) as *mut Block;
                unsafe {
                    if (*b).class == CLASS_NONE {
                        // untouched, or pooled by an earlier sweep (which
                        // resets the class): nothing to sweep here. A pooled
                        // block was used before and is empty now.
                        if (*b).owner == OWNER_EMPTY {
                            chunk_touched = true;
                        }
                        continue;
                    }
                    // Rebuild the starts bitmap from the survivors only: a
                    // freed-but-unreallocated slot must never be swept again
                    // (its fields hold freelist pointers) nor stay
                    // resolvable. Same for the free list: exactly the dead
                    // slots of this round.
                    let old_starts = (*b).starts;
                    (*b).starts = [0; mm_heap::START_WORDS];
                    (*b).freelist = std::ptr::null_mut();
                    let mut live: u32 = 0;
                    let csize = SIZES[(*b).class as usize] as u64;
                    for w in 0..mm_heap::START_WORDS {
                        let mut s = old_starts[w];
                        while s != 0 {
                            let g = w * 64 + s.trailing_zeros() as usize;
                            let p = (*b).data_ptr().add(g * 16);
                            let f = get_flags(p);
                            if f & (flags::MARK | flags::IMMORTAL) != 0 {
                                if f & flags::MARK != 0 {
                                    set_flags(p, f & !flags::MARK);
                                }
                                live += 1;
                                live_bytes += csize;
                                (*b).start_set(g);
                            } else {
                                let mut dec = |c: *mut u8| com_decref(c);
                                for_each_child(p, &mut dec);
                                *((p as *mut *mut u8).add(1)) = (*b).freelist;
                                (*b).freelist = p;
                                freed += 1;
                            }
                            s &= s - 1;
                        }
                    }
                    (*b).live_objects = live;
                    if live == 0 {
                        if (*b).owner == OWNER_EMPTY {
                            // already in the empty pool: must not be pushed
                            // again (a duplicate would hand one block to two
                            // size classes)
                            chunk_touched = true;
                            continue;
                        }
                        if (*b).bump_next == 0 && (*b).sticky_listed == 0
                            && (*b).owner == OWNER_NONE
                        {
                            continue; // untouched: not our memory to pool
                        }
                        // fully dead: recycle to the heap-wide pool
                        (*b).owner = OWNER_EMPTY;
                        (*b).class = CLASS_NONE;
                        (*b).bump_next = 0;
                        (*b).sticky_listed = 0;
                        (*b).starts = [0; mm_heap::START_WORDS];
                        (*b).freelist = std::ptr::null_mut();
                        to_pool.push(b);
                        chunk_touched = true;
                        continue;
                    }
                    chunk_dead = false;
                    chunk_touched = true;
                    // partially free and not on a list: make it findable
                    if !(*b).freelist.is_null() && (*b).owner == OWNER_NONE {
                        to_list.push(((*b).class as usize, b));
                    }
                }
            }
            if chunk_dead && chunk_touched {
                // every claimed block of the chunk is empty now (the rest
                // were never touched): release the chunk, purging its
                // pooled blocks from the pool below
                dead_small_chunks.push((idx, c.base, c.end));
            }
        }

        for (ci, b) in to_list {
            unsafe {
                (*b).owner = OWNER_LIST;
                (*b).sticky_listed = 1;
            }
            self.heap.classes[ci].blocks.push(b);
        }
        for b in to_pool {
            unsafe { (*b).owner = OWNER_EMPTY }
            self.heap.empty_pool.push(b);
        }
        // any pooled or listed block inside a released small chunk is
        // dangling: purge it from every index
        for &(_, base, end) in &dead_small_chunks {
            self.heap
                .empty_pool
                .retain(|&b| !(b as usize >= base && (b as usize) < end));
            for cs in &mut self.heap.classes {
                cs.blocks
                    .retain(|&b| !(b as usize >= base && (b as usize) < end));
            }
        }
        // release dead large chunks and fully-dead small chunks (highest
        // first so removal indices stay valid)
        let mut release: Vec<usize> = dead_chunks.clone();
        release.extend(dead_small_chunks.iter().map(|&(idx, _, _)| idx));
        release.sort_unstable();
        release.dedup();
        for &idx in release.iter().rev() {
            let c = self.heap.chunks[idx];
            let total = c.end - c.base;
            unsafe {
                std::alloc::dealloc(
                    c.base as *mut u8,
                    std::alloc::Layout::from_size_align(total, BLOCK).unwrap(),
                );
            }
            self.heap.reserved -= total;
            self.heap.chunks.remove(idx);
        }
        if !self.heap.chunks.is_empty() {
            self.heap.min_addr = self.heap.chunks.first().unwrap().base;
            self.heap.max_addr = self.heap.chunks.last().unwrap().end;
        } else {
            self.heap.min_addr = usize::MAX;
            self.heap.max_addr = 0;
        }
        // cursors may point at pooled/released blocks; the lists are
        // authoritative after a sweep
        for cs in &mut self.heap.classes {
            cs.cur = std::ptr::null_mut();
        }

        self.stats.frees += freed;
        self.live_bytes = live_bytes;
        self.live_objects = live_objects;
        self.stats.live_bytes = live_bytes;
        self.stats.live_objects = live_objects;
        self.alloc_since_gc = 0;
        self.gc_threshold = ((live_bytes as usize) * 2).max(gc_min());
    }

    fn collect_internal(&mut self) -> u64 {
        let t0 = Instant::now();
        let g = self as *mut GcMem;
        unsafe {
            // precise roots: the protection stack and the global frame
            let prot: Vec<*mut u8> = self.protect_stack.clone();
            for p in prot {
                mark_obj(g, p);
            }
            mark_obj(g, self.global_env);
            mm_gc_spill(scan_and_mark, g as *mut u8);
            self.prune_chars();
        }
        self.sweep();
        let dt = t0.elapsed().as_nanos() as u64;
        self.stats.collections += 1;
        self.stats.collect_total_ns += dt;
        self.stats.collect_max_pause_ns = self.stats.collect_max_pause_ns.max(dt);
        self.last_pause_ns = dt;
        dt
    }
}

impl Default for GcMem {
    fn default() -> Self {
        Self::new()
    }
}

/// Enumerate every child pointer (attrs + fields + payload slots).
unsafe fn for_each_child<F: FnMut(*mut u8)>(p: *mut u8, f: &mut F) {
    let a = rd_ptr(p, 8);
    if !a.is_null() {
        f(a);
    }
    let t = *(p as *const u8);
    match t {
        LISTSXP | LANGSXP | CLOSXP => {
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

fn elem_size(t: u8) -> usize {
    match t {
        REALSXP => 8,
        INTSXP | LGLSXP => 4,
        _ => 8,
    }
}

impl Mem for GcMem {
    type Handle = GcSexp;

    fn create() -> Self {
        GcMem::new()
    }
    fn nil(&self) -> GcSexp {
        GcSexp(self.nil_p)
    }
    #[inline]
    fn retain(&self, h: &GcSexp) -> GcSexp {
        *h
    }
    #[inline]
    fn discard(&mut self, h: GcSexp) {
        let _ = h;
    }

    fn alloc_real(&mut self, len: usize) -> GcSexp {
        GcSexp(self.vec_alloc(REALSXP, len, len))
    }
    fn alloc_int(&mut self, len: usize) -> GcSexp {
        GcSexp(self.vec_alloc(INTSXP, len, len))
    }
    fn alloc_generic(&mut self, len: usize) -> GcSexp {
        let cap = len.max(4).next_power_of_two();
        GcSexp(self.vec_alloc(VECSXP, len, cap))
    }
    fn alloc_string(&mut self, len: usize) -> GcSexp {
        GcSexp(self.vec_alloc(STRSXP, len, len))
    }

    fn cons(&mut self, car: &GcSexp, cdr: &GcSexp, tag: &GcSexp, lang: bool) -> GcSexp {
        self.cons_alloc(car, cdr, tag, lang)
    }

    fn env_new(&mut self, parent: &GcSexp) -> GcSexp {
        GcSexp(self.env_alloc(parent))
    }

    fn closure_new(&mut self, formals: &GcSexp, body: &GcSexp, env: &GcSexp) -> GcSexp {
        let p = self.alloc_object(CLOSXP, 40);
        unsafe {
            wr_ptr(p, OFF_FIELDS, formals.0);
            wr_ptr(p, OFF_FIELDS + 8, body.0);
            wr_ptr(p, OFF_FIELDS + 16, env.0);
            com_incref(formals.0);
            com_incref(body.0);
            com_incref(env.0);
        }
        GcSexp(p)
    }

    fn char_new(&mut self, bytes: &[u8]) -> GcSexp {
        GcSexp(self.char_alloc(bytes))
    }

    fn char_intern(&mut self, bytes: &[u8]) -> GcSexp {
        let hash = hash_bytes(bytes) as u64;
        if let Some(bucket) = self.chars.get(&hash) {
            for &q in bucket {
                unsafe {
                    if rd_u32(q, OFF_FIELDS + 4) as usize == bytes.len() {
                        let qb = q.add(OFF_FIELDS + 8);
                        if std::slice::from_raw_parts(qb, bytes.len()) == bytes {
                            return GcSexp(q); // no count: the table is weak
                        }
                    }
                }
            }
        }
        let p = self.char_alloc(bytes);
        unsafe {
            set_flags(p, get_flags(p) | flags::INTERNED);
        }
        self.chars.entry(hash).or_default().push(p);
        self.stats.chars_interned += 1;
        GcSexp(p)
    }

    fn symbol_intern(&mut self, name: &[u8]) -> GcSexp {
        let hash = hash_bytes(name) as u64;
        if let Some(bucket) = self.syms.get(&hash) {
            for &q in bucket {
                unsafe {
                    let nm = rd_ptr(q, OFF_FIELDS);
                    if rd_u32(nm, OFF_FIELDS + 4) as usize == name.len() {
                        let nb = nm.add(OFF_FIELDS + 8);
                        if std::slice::from_raw_parts(nb, name.len()) == name {
                            return GcSexp(q);
                        }
                    }
                }
            }
        }
        let nm = self.char_alloc(name);
        unsafe {
            set_flags(nm, flags::IMMORTAL); // symbols never die
        }
        let p = self.alloc_object(SYMSXP, 32);
        unsafe {
            wr_ptr(p, OFF_FIELDS, nm);
            wr_u32(p, OFF_FIELDS + 8, hash as u32);
            set_flags(p, flags::IMMORTAL);
        }
        self.syms.entry(hash).or_default().push(p);
        GcSexp(p)
    }

    fn int_scalar(&mut self, v: i32) -> GcSexp {
        if (-1024..8192).contains(&v) {
            return GcSexp(self.int_cache[(v + 1024) as usize]);
        }
        GcSexp(self.int_alloc(v))
    }

    fn real_scalar(&mut self, v: f64) -> GcSexp {
        let p = self.vec_alloc(REALSXP, 1, 1);
        unsafe { *(p.add(OFF_FIELDS + 8) as *mut f64) = v }
        GcSexp(p)
    }

    fn sexp_type(&self, v: &GcSexp) -> u8 {
        unsafe { *(v.0 as *const u8) }
    }
    fn real_data(&self, v: &GcSexp) -> *mut f64 {
        unsafe { v.0.add(OFF_FIELDS + 8) as *mut f64 }
    }
    fn real_len(&self, v: &GcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }
    fn int_data(&self, v: &GcSexp) -> *mut i32 {
        unsafe { v.0.add(OFF_FIELDS + 8) as *mut i32 }
    }
    fn int_len(&self, v: &GcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }
    fn scalar_int_value(&self, v: &GcSexp) -> i32 {
        unsafe { *(v.0.add(OFF_FIELDS + 8) as *const i32) }
    }

    fn generic_elt(&self, v: &GcSexp, i: usize) -> GcSexp {
        GcSexp(unsafe { rd_ptr(v.0, OFF_FIELDS + 8 + i * 8) })
    }
    fn set_generic_elt(&mut self, v: &GcSexp, i: usize, val: &GcSexp) {
        unsafe {
            let slot = v.0.add(OFF_FIELDS + 8 + i * 8);
            let old = *(slot as *mut *mut u8);
            com_incref(val.0);
            *(slot as *mut *mut u8) = val.0;
            com_decref(old);
        }
    }
    fn generic_len(&self, v: &GcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS) as usize }
    }

    fn string_elt(&self, v: &GcSexp, i: usize) -> GcSexp {
        self.generic_elt(v, i)
    }
    fn set_string_elt(&mut self, v: &GcSexp, i: usize, val: &GcSexp) {
        self.set_generic_elt(v, i, val)
    }
    fn string_len(&self, v: &GcSexp) -> usize {
        self.generic_len(v)
    }
    fn char_bytes(&self, c: &GcSexp) -> &[u8] {
        unsafe {
            let l = rd_u32(c.0, OFF_FIELDS + 4) as usize;
            std::slice::from_raw_parts(c.0.add(OFF_FIELDS + 8), l)
        }
    }

    fn car(&self, v: &GcSexp) -> GcSexp {
        GcSexp(unsafe { rd_ptr(v.0, OFF_FIELDS) })
    }
    fn cdr(&self, v: &GcSexp) -> GcSexp {
        GcSexp(unsafe { rd_ptr(v.0, OFF_FIELDS + 8) })
    }
    fn tag(&self, v: &GcSexp) -> GcSexp {
        GcSexp(unsafe { rd_ptr(v.0, OFF_FIELDS + 16) })
    }
    fn set_car(&mut self, v: &GcSexp, val: &GcSexp) {
        unsafe {
            let old = rd_ptr(v.0, OFF_FIELDS);
            com_incref(val.0);
            wr_ptr(v.0, OFF_FIELDS, val.0);
            com_decref(old);
        }
    }

    fn env_define(&mut self, env: &GcSexp, sym: &GcSexp, val: &GcSexp) {
        unsafe {
            let e = env.0;
            let n = rd_u32(e, OFF_FIELDS + 16);
            if n < ENV_SLOTS as u32 {
                wr_ptr(e, OFF_FIELDS + 24 + n as usize * 16, sym.0);
                wr_ptr(e, OFF_FIELDS + 24 + n as usize * 16 + 8, val.0);
                wr_u32(e, OFF_FIELDS + 16, n + 1);
                com_incref(val.0);
                return;
            }
            let cell = self.cons_alloc(val, &GcSexp(self.nil_p), sym, false);
            let cp = cell.0;
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

    fn env_set(&mut self, env: &GcSexp, sym: &GcSexp, val: &GcSexp) {
        unsafe {
            let e = env.0;
            let n = rd_u32(e, OFF_FIELDS + 16);
            for i in 0..n as usize {
                if rd_ptr(e, OFF_FIELDS + 24 + i * 16) == sym.0 {
                    let slot = e.add(OFF_FIELDS + 24 + i * 16 + 8);
                    let old = *(slot as *mut *mut u8);
                    com_incref(val.0);
                    *(slot as *mut *mut u8) = val.0;
                    com_decref(old);
                    return;
                }
            }
            let mut cell = rd_ptr(e, OFF_FIELDS + 8);
            while cell != self.nil_p {
                if rd_ptr(cell, OFF_FIELDS + 16) == sym.0 {
                    let old = rd_ptr(cell, OFF_FIELDS);
                    com_incref(val.0);
                    wr_ptr(cell, OFF_FIELDS, val.0);
                    com_decref(old);
                    return;
                }
                cell = rd_ptr(cell, OFF_FIELDS + 8);
            }
        }
    }

    fn env_take(&mut self, env: &GcSexp, sym: &GcSexp) -> Option<GcSexp> {
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
                    // the binding's heap reference disappears: the caller
                    // gets an uncounted stack pointer, and copy-on-modify
                    // decisions need the count to be exact *now*
                    com_decref(val);
                    return Some(GcSexp(val));
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
                    // the unlink creates a new chain edge to `next` (the
                    // cell's count on it vanishes with the cell); the
                    // binding's count on `val` transfers to the caller
                    com_incref(next);
                    com_decref(cell);
                    com_decref(val);
                    // clear the fields so the sweeper does not decrement
                    // the transferred counts again when it frees the cell
                    wr_ptr(cell, OFF_FIELDS, self.nil_p);
                    wr_ptr(cell, OFF_FIELDS + 8, self.nil_p);
                    wr_ptr(cell, OFF_FIELDS + 16, self.nil_p);
                    return Some(GcSexp(val));
                }
                prev = cell;
                cell = rd_ptr(cell, OFF_FIELDS + 8);
            }
            None
        }
    }

    fn env_lookup(&mut self, env: &GcSexp, sym: &GcSexp) -> Option<GcSexp> {
        unsafe { self.env_find(env.0, sym.0).map(GcSexp) }
    }

    fn global_env(&self) -> GcSexp {
        GcSexp(self.global_env)
    }

    fn attrs_set(&mut self, obj: &GcSexp, attrs: &GcSexp) {
        unsafe {
            let old = rd_ptr(obj.0, 8);
            com_incref(attrs.0);
            wr_ptr(obj.0, 8, attrs.0);
            com_decref(old);
        }
    }
    fn attrs_get(&self, obj: &GcSexp) -> GcSexp {
        GcSexp(unsafe { rd_ptr(obj.0, 8) })
    }

    fn external_refs(&self, h: &GcSexp) -> u32 {
        let c = unsafe { get_com(h.0) };
        if c >= SAT {
            0
        } else {
            c
        }
    }

    fn duplicate(&mut self, h: &GcSexp) -> GcSexp {
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
                    if !attrs.is_null() {
                        com_incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    GcSexp(q)
                }
                STRSXP | VECSXP => {
                    let q = self.vec_alloc(t, len, len);
                    for i in 0..len {
                        let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                        com_incref(c);
                        wr_ptr(q, OFF_FIELDS + 8 + i * 8, c);
                    }
                    if !attrs.is_null() {
                        com_incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    GcSexp(q)
                }
                ENVSXP | CHARSXP | SYMSXP | NILSXP => *h,
                _ => {
                    let q = self.alloc_object(t, 40);
                    for off in [OFF_FIELDS, OFF_FIELDS + 8, OFF_FIELDS + 16] {
                        let c = rd_ptr(p, off);
                        com_incref(c);
                        wr_ptr(q, off, c);
                    }
                    if !attrs.is_null() {
                        com_incref(attrs);
                        wr_ptr(q, 8, attrs);
                    }
                    GcSexp(q)
                }
            }
        }
    }

    fn vector_push(&mut self, v: &GcSexp, val: &GcSexp) -> GcSexp {
        unsafe {
            let p = v.0;
            let len = rd_u32(p, OFF_FIELDS) as usize;
            let cap = rd_u32(p, OFF_FIELDS + 4) as usize;
            if len < cap {
                let slot = p.add(OFF_FIELDS + 8 + len * 8);
                let old = *(slot as *mut *mut u8);
                com_incref(val.0);
                *(slot as *mut *mut u8) = val.0;
                com_decref(old);
                wr_u32(p, OFF_FIELDS, (len + 1) as u32);
                return *v;
            }
            let new_cap = (cap * 2).max(4);
            let q = self.vec_alloc(VECSXP, len + 1, new_cap);
            for i in 0..len {
                let c = rd_ptr(p, OFF_FIELDS + 8 + i * 8);
                com_incref(c);
                wr_ptr(q, OFF_FIELDS + 8 + i * 8, c);
            }
            com_incref(val.0);
            wr_ptr(q, OFF_FIELDS + 8 + len * 8, val.0);
            let attrs = rd_ptr(p, 8);
            if !attrs.is_null() {
                com_incref(attrs);
                wr_ptr(q, 8, attrs);
            }
            GcSexp(q)
        }
    }

    fn vector_capacity(&self, v: &GcSexp) -> usize {
        unsafe { rd_u32(v.0, OFF_FIELDS + 4) as usize }
    }

    fn collect(&mut self) -> u64 {
        self.collect_internal()
    }

    fn stats(&self) -> Stats {
        let mut s = self.stats.clone();
        s.protected_now = self.protect_stack.len();
        s.live_bytes = self.live_bytes;
        s.live_objects = self.live_objects;
        s
    }

    fn protect(&mut self, h: &GcSexp) -> GcSexp {
        self.protect_stack.push(h.0);
        *h
    }
    fn unprotect(&mut self, n: usize) {
        let len = self.protect_stack.len();
        self.protect_stack.truncate(len.saturating_sub(n));
    }

    fn heap_reserved(&self) -> usize {
        self.heap.reserved
    }
}

impl GcMem {
    pub fn set_stack_top(&mut self, top: usize) {
        if top > self.stack_top {
            self.stack_top = top;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locals_found_conservatively() {
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        let v = m.alloc_real(1000);
        for i in 0..1000 {
            // churn enough to force collections
            let w = m.alloc_real(10);
            unsafe { *m.real_data(&w) = i as f64 };
            m.discard(w);
        }
        m.collect();
        assert_eq!(m.real_len(&v), 1000); // v survived on the stack alone
    }

    #[test]
    fn interior_pointers_resolve() {
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        let v = m.alloc_real(100_000);
        let d = m.real_data(&v);
        for _ in 0..20 {
            let w = m.alloc_real(10);
            m.discard(w);
        }
        m.collect();
        unsafe { *d.add(99999) = 42.0 };
        assert_eq!(unsafe { *m.real_data(&v).add(99999) }, 42.0);
    }

    #[test]
    fn cycles_collected() {
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        let nil = m.nil();
        for _ in 0..10_000 {
            let e = m.env_new(&m.global_env());
            let c = m.closure_new(&nil, &nil, &e);
            let s = m.symbol_intern(b"g");
            m.env_define(&e, &s, &c);
        }
        m.collect();
        let s = m.stats();
        assert!(s.live_objects < 500, "live objects: {}", s.live_objects);
    }

    #[test]
    fn recursive_envs_survive() {
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        fn rec(m: &mut GcMem, depth: i32) -> i64 {
            let e = m.env_new(&m.global_env());
            let s = m.symbol_intern(b"n");
            let nv = m.int_scalar(depth);
            m.env_define(&e, &s, &nv);
            let r = if depth == 0 {
                m.collect(); // collections deep in the recursion
                0
            } else {
                rec(m, depth - 1)
            };
            // e must still be alive after the collections below
            let v = m.env_lookup(&e, &s).unwrap();
            m.scalar_int_value(&v) as i64 + r
        }
        let r = rec(&mut m, 200);
        assert_eq!(r, (0..=200).sum::<i32>() as i64);
    }

    #[test]
    fn fib_shaped_envs_survive() {
        // one env per call, allocations triggering collections mid-recursion:
        // exactly the fib workload's shape. Run with MM_GC_MIN=65536 to
        // force collections; survives either way.
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        fn fib(m: &mut GcMem, n: i32, g: &GcSexp) -> i64 {
            let e = m.env_new(g);
            if n == 15 {
                WATCH.store(e.0 as usize, std::sync::atomic::Ordering::Relaxed);
            }
            let s = m.symbol_intern(b"n");
            let nv = m.int_scalar(n);
            m.env_define(&e, &s, &nv);
            let r = if n < 2 {
                n as i64
            } else {
                let a = fib(m, n - 1, g);
                let b = fib(m, n - 2, g);
                a + b
            };
            let v = match m.env_lookup(&e, &s) {
                Some(v) => v,
                None => {
                    eprintln!("WATCH lookup FAILED for e={:?} n={}", e.0, n);
                    return -1000;
                }
            };
            m.scalar_int_value(&v) as i64 + r
        }
        let g = m.global_env();
        let r = fib(&mut m, 20, &g);
        let st = m.stats();
        eprintln!(
            "STATS allocs={} alloc_mb={:.1} cols={} thr={} live_objs={} live_mb={:.2}",
            st.allocs,
            st.bytes_allocated as f64 / 1048576.0,
            st.collections,
            m.gc_threshold,
            st.live_objects,
            st.live_bytes as f64 / 1048576.0,
        );
        m.collect();
        let st = m.stats();
        eprintln!(
            "AFTER top-level collect: live_objs={} live_mb={:.2}",
            st.live_objects,
            st.live_bytes as f64 / 1048576.0,
        );
        assert_eq!(r, 6765 + (21..=41).sum::<i64>());
    }

    #[test]
    fn protect_stack_is_a_root() {
        let mut m = GcMem::new();
        m.set_stack_top(&m as *const _ as usize);
        let v = m.alloc_real(10);
        let p = m.protect(&v);
        let _ = v;
        let mut sink = 0u64;
        // churn below a fresh call frame so no stale copy of `v` remains
        fn churn(m: &mut GcMem, n: usize, p: &GcSexp) -> u64 {
            let keep = m.protect(p);
            let mut s = 0u64;
            for i in 0..n {
                let w = m.alloc_real(10);
                s += unsafe { *m.real_data(&w) = i as f64; i as u64 };
                m.discard(w);
            }
            m.unprotect(1);
            s + m.real_len(&keep) as u64
        }
        sink += churn(&mut m, 1000, &p);
        m.collect();
        assert_eq!(m.real_len(&p), 10);
        assert_eq!(sink, 499_500 + 10); // sum(0..1000) + length of the protected vector
        m.unprotect(1);
    }
}
