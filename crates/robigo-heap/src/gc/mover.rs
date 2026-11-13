//! `ROBIGO_GC_MOVE=1` (docs/g3.md §3 "Checks", §16; m1-plan H3): a debug
//! collector that moves every object it may at every collection and poisons
//! the old copy, so a raw `Sexp` kept across a safepoint outside the roots
//! (the bug the borrow checker cannot see in the raw layer, §2.2) reads
//! poison or faults at its next use instead of silently reading a live
//! object.
//!
//! A collection marks as always (precise roots, weak references, C
//! segments as pins); then, before the sweep:
//!
//! 1. **evacuate**: every marked object that may move gets a new copy, and
//!    the old one a forwarding header ([`FORWARDED`] and the new address at
//!    +8). What may not move: immortal objects and every object ever handed
//!    to C (the sticky [`crate::extra::SEEN_BY_C`], which a C word's pin
//!    sets too: C may keep any of them where no collection looks);
//! 2. **forward** every edge through the same [`Visitor`] the marker uses:
//!    the embedder's roots, the heap's own lists (precious set, weak
//!    references and their fields, the finalizer queue, the barrier's
//!    log), the CHARSXP cache (in place: a slot's position is its content
//!    hash, which moving does not change), and the fields of every live
//!    object, immortal ones included;
//! 3. **retire** the old copies: a small one is poisoned ([`POISON_TYPE`],
//!    no flags, `0xDB` everywhere) and freed by the sweep that follows; a
//!    large one keeps its chunk out of the heap, `mprotect`ed to no access,
//!    until the next collection, so a stale access faults.
//!
//! The heap sizes do not change: a copy is the object, not an allocation
//! (its cells stay counted once, and the poisoned old copy counts none).

use super::*;

/// The type byte of an old copy while a collection forwards edges: the new
/// address is at +8. Never seen outside a collection.
const FORWARDED: u8 = 0xFE;

/// The type byte of a retired old copy (and, under gctorture, of a freed
/// slot): no SEXPTYPE has it, and the rest of the object is `0xDB` bytes,
/// so a stale `Sexp` reads garbage types and pointers and fails loudly.
pub const POISON_TYPE: u8 = 0xDB;

/// What [`Core::evacuate`] moved, for the forwarding and the retirement.
pub(super) struct Moved {
    /// old small copies
    small: Vec<*mut u8>,
    /// old large chunks (their objects are forwarded)
    large: Vec<usize>,
}

/// The forwarding visitor: an edge to an old copy takes the new address.
struct Forward;

impl Visitor for Forward {
    #[inline]
    fn edge(&mut self, e: &mut Sexp) {
        let x = *e;
        if x.is_null() {
            return;
        }
        // SAFETY: an edge is null, immortal or a live object (the rules of
        // the raw setters, and the collection's caller vouches for the
        // roots); nothing is swept yet, so its header is readable, and an
        // old copy keeps its new address at +8 until it is retired.
        unsafe {
            if x.typ() == FORWARDED {
                *e = Sexp(*(x.0.add(8) as *const *mut u8));
            }
        }
    }
}

impl Core {
    /// Whether collections move objects (`ROBIGO_GC_MOVE=1`, read when the
    /// core is made, or [`Core::set_gc_move`]).
    pub fn gc_move(&self) -> bool {
        self.gc_move
    }

    /// Move objects at every collection from now on, or stop. Turn it on
    /// before anything is handed to C: what C has seen is recorded only
    /// while moving is on (`ROBIGO_GC_MOVE=1` sets it when the core is made),
    /// so an object C kept from before could move.
    pub fn set_gc_move(&mut self, on: bool) {
        self.gc_move = on;
    }

    /// Under `gc_move`, record a C word's object as one C has seen (the
    /// sticky [`crate::extra::SEEN_BY_C`]): it stays where it is, in this
    /// collection and every later one, since C may keep its address.
    ///
    /// # Safety
    /// During a collection's segment scan; `p` an allocated object.
    #[inline]
    pub(super) unsafe fn pin(&mut self, p: Sexp) {
        if self.gc_move {
            // SAFETY: the caller's contract: an allocated object's header.
            unsafe { p.mark_seen_by_c() };
        }
    }

    /// Give the large chunks retired by the last collection back (they were
    /// `mprotect`ed to no access).
    pub(super) fn release_quarantine(&mut self) {
        for c in std::mem::take(&mut self.quarantine) {
            // SAFETY: the chunk was allocated with this layout and taken out
            // of the heap when its object moved; nothing may use it (that is
            // what its protection checked). Access is given back before the
            // allocator, which may write its own bookkeeping, sees it again.
            unsafe {
                protect(c.base, c.end - c.base, libc::PROT_READ | libc::PROT_WRITE);
                std::alloc::dealloc(c.base as *mut u8, c.layout());
            }
            self.heap.reserved -= c.end - c.base;
        }
    }

    /// Copy every marked object that may move, leaving a forwarding header
    /// in its old copy; the copies are marked, so the sweep keeps them.
    ///
    /// # Safety
    /// During a collection, after every mark (precise, weak, pins) and the
    /// CHARSXP prune, before any forwarding or the sweep.
    pub(super) unsafe fn evacuate(&mut self) -> Moved {
        let mut small = Vec::new();
        let mut large = Vec::new();
        // first the list, then the copies: a copy allocates, which changes
        // the blocks and chunks walked here
        for c in self.heap.chunks.clone() {
            if c.large {
                let p = (c.base + DATA_OFF) as *mut u8;
                // SAFETY: a large chunk's object and its mark word.
                if unsafe { *(c.base as *const u64).add(1) } == self.epoch && unsafe { self.may_move(p) } {
                    large.push(c.base);
                }
                continue;
            }
            for bi in 0..c.claimed as usize {
                let b = (c.base + bi * BLOCK) as *mut Block;
                // SAFETY: a claimed block's initialized header; a start bit
                // marks an object's first byte.
                unsafe {
                    if (*b).class == CLASS_NONE {
                        continue;
                    }
                    for w in 0..START_WORDS {
                        let mut s = (*b).starts[w] & (*b).marks[w];
                        while s != 0 {
                            let g = w * 64 + s.trailing_zeros() as usize;
                            let p = Block::data_ptr(b).add(g * 16);
                            if self.may_move(p) {
                                small.push(p);
                            }
                            s &= s - 1;
                        }
                    }
                }
            }
        }
        let mut moved = Moved { small: Vec::with_capacity(small.len()), large: Vec::with_capacity(large.len()) };
        for p in small {
            // SAFETY: `p` is a marked small object; its slot is its class's
            // size, all of which is copied into a fresh slot of the class.
            unsafe {
                let b = (p as usize & !(BLOCK - 1)) as *mut Block;
                let size = SIZES[(*b).class as usize];
                let q = self.heap.alloc(size);
                std::ptr::copy_nonoverlapping(p, q, size);
                self.mark_copy(q);
                forward_to(p, q);
            }
            moved.small.push(p);
        }
        for base in large {
            let Some(i) = self.heap.chunks.iter().position(|c| c.base == base) else { continue };
            let c = self.heap.chunks[i];
            let p = (base + DATA_OFF) as *mut u8;
            let size = c.end - base - DATA_OFF;
            // (a refused copy stays where it is: a debug collector degrades)
            let Some(q) = self.heap.try_alloc_large(size) else { continue };
            // SAFETY: both are large objects' payloads of `size` bytes in
            // different chunks; the copy's chunk header gets this
            // collection's mark.
            unsafe {
                std::ptr::copy_nonoverlapping(p, q, size);
                *((q as usize - DATA_OFF) as *mut u64).add(1) = self.epoch;
                forward_to(p, q);
            }
            moved.large.push(base);
        }
        self.stats.moved += (moved.small.len() + moved.large.len()) as u64;
        moved
    }

    /// Whether the marked object `p` may move.
    ///
    /// # Safety
    /// `p` is an allocated object.
    unsafe fn may_move(&self, p: *mut u8) -> bool {
        let x = Sexp(p);
        // SAFETY: the caller's contract.
        unsafe { !x.has_flag(flags::IMMORTAL) && !x.seen_by_c() }
    }

    /// Mark a copy without tracing it (its fields are forwarded instead).
    ///
    /// # Safety
    /// `q` is a fresh small object of this heap.
    unsafe fn mark_copy(&mut self, q: *mut u8) {
        // SAFETY: as in `mark_obj`, for a small object.
        unsafe {
            let b = (q as usize & !(BLOCK - 1)) as *mut Block;
            let g = (q as usize - Block::data_ptr(b) as usize) / 16;
            (*b).marks[g >> 6] |= 1u64 << (g & 63);
            (*b).marked = (*b).marked.wrapping_add(1);
        }
    }

    /// Forward the heap's own edges and every live object's fields.
    ///
    /// # Safety
    /// During a collection, after [`Core::evacuate`], before the
    /// retirement.
    pub(super) unsafe fn forward_heap(&mut self) {
        let f = &mut Forward;
        // SAFETY: every list holds live objects (the collection marked them)
        // and each weak reference is a standard vector of object words;
        // `for_each_marked` yields live objects, whose fields are edges.
        unsafe {
            for x in self.precious.iter_mut().chain(self.weak_refs.iter_mut()).chain(self.finalize_queue.iter_mut()) {
                f.edge(x);
            }
            for x in (*self.remembered.0.get()).iter_mut() {
                f.edge(x);
            }
            self.chars.forward(|x| f.edge(x));
            self.for_each_marked(|p| {
                let x = Sexp(p);
                if x.typ() == FORWARDED {
                    return;
                }
                x.trace_fields(f);
                // a weak reference's fields are the weak pass's, not
                // `trace_fields`'s: forwarded here, live or not
                if x.typ() == WEAKREFSXP && !x.has_flag(flags::ALTREP | flags::EXTDATA) {
                    let d = x.std_data() as *mut Sexp;
                    for i in 0..x.std_len() as usize {
                        f.edge(&mut *d.add(i));
                    }
                }
            });
        }
    }

    /// Forward the embedder's roots.
    ///
    /// # Safety
    /// As for [`Core::forward_heap`]; `roots` are the collection's.
    pub(super) unsafe fn forward_roots<R: Trace + ?Sized>(roots: &mut R) {
        roots.trace(&mut Forward);
    }

    /// Poison the old small copies and leave them to the sweep; take the
    /// old large chunks out of the heap, without access, until the next
    /// collection.
    ///
    /// # Safety
    /// During a collection, after every edge is forwarded.
    pub(super) unsafe fn retire(&mut self, moved: Moved) {
        for p in moved.small {
            // SAFETY: `p` is an old copy in a block: its class-sized slot is
            // ours to overwrite (nothing refers to it any more), and its mark
            // bit is cleared so the sweep frees it, with no flags (no release
            // call, no immortality) and no vector cells counted.
            unsafe {
                let b = (p as usize & !(BLOCK - 1)) as *mut Block;
                let size = SIZES[(*b).class as usize];
                std::ptr::write_bytes(p, 0xDB, size);
                *p.add(1) = 0;
                let g = (p as usize - Block::data_ptr(b) as usize) / 16;
                (*b).marks[g >> 6] &= !(1u64 << (g & 63));
                (*b).marked = (*b).marked.wrapping_sub(1);
            }
        }
        for base in moved.large {
            let Some(i) = self.heap.chunks.iter().position(|c| c.base == base) else { continue };
            let c = self.heap.chunks.remove(i);
            // SAFETY: the chunk is out of the heap's list (the sweep and the
            // resolver no longer see it) and nothing refers to its object;
            // the region is whole pages of the chunk's own allocation.
            unsafe { protect(c.base, c.end - c.base, libc::PROT_NONE) };
            self.quarantine.push(c);
        }
    }

    /// Every marked or immortal object's first byte (old copies included:
    /// their mark bit is still set).
    fn for_each_marked(&self, mut f: impl FnMut(*mut u8)) {
        let epoch = self.epoch;
        for c in &self.heap.chunks {
            if c.large {
                let p = (c.base + DATA_OFF) as *mut u8;
                // SAFETY: a large chunk's mark word and its object's flags.
                if unsafe { *(c.base as *const u64).add(1) == epoch || *p.add(1) & flags::IMMORTAL != 0 } {
                    f(p);
                }
                continue;
            }
            for bi in 0..c.claimed as usize {
                let b = (c.base + bi * BLOCK) as *mut Block;
                // SAFETY: a claimed block's initialized header; a start bit
                // marks an object's first byte.
                unsafe {
                    if (*b).class == CLASS_NONE {
                        continue;
                    }
                    for w in 0..START_WORDS {
                        let mut s = (*b).starts[w];
                        while s != 0 {
                            let g = w * 64 + s.trailing_zeros() as usize;
                            let p = Block::data_ptr(b).add(g * 16);
                            if (*b).marks[w] & (1u64 << (g & 63)) != 0 || *p.add(1) & flags::IMMORTAL != 0 {
                                f(p);
                            }
                            s &= s - 1;
                        }
                    }
                }
            }
        }
    }
}

/// Give the old copy `p` a forwarding header to `q`, and no flags: the
/// copy is the object now (an EXTDATA release call is the copy's alone,
/// even if the heap is dropped before the old copy is retired).
///
/// # Safety
/// `p` is an object (at least 16 bytes) that nothing reads as one again
/// before its retirement but [`Forward`].
unsafe fn forward_to(p: *mut u8, q: *mut u8) {
    // SAFETY: the caller's contract.
    unsafe {
        *p = FORWARDED;
        *p.add(1) = 0;
        *(p.add(8) as *mut *mut u8) = q;
    }
}

/// `mprotect` whole pages of a chunk; a failure (which only a corrupt
/// address could cause) stops the process rather than let a debug check
/// silently not check.
///
/// # Safety
/// `[base, base + len)` is a page-aligned region of a chunk this heap
/// allocated.
unsafe fn protect(base: usize, len: usize, prot: libc::c_int) {
    // SAFETY: the caller's contract.
    if unsafe { libc::mprotect(base as *mut libc::c_void, len, prot) } != 0 {
        eprintln!("robigo: ROBIGO_GC_MOVE could not change a retired chunk's protection");
        std::process::abort();
    }
}
