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
//!    the old one a forwarding header ([`FORWARDED`] and the new address in
//!    its link word, `layout::header::OFF_LINK`). What may not move: immortal objects and every object ever handed
//!    to C (the sticky [`crate::extra::SEEN_BY_C`], which a C word's pin
//!    sets too: C may keep any of them where no collection looks);
//! 2. **forward** every edge through the same [`Visitor`] the marker uses:
//!    the embedder's roots, the heap's own lists (precious set, weak
//!    references and their fields, the barrier's
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
//!
//! `ROBIGO_GC_MOVE=young:N` (H8) is the same check at a torture
//! collection's cost: every N-th moving collection moves everything as
//! above, and the others move only the **young** objects, those no
//! collection has moved yet ([`OLD`] clear: every fresh object). Such a
//! collection takes the pins first ([`Core::pin_segments`]), then marks as
//! always, but the marker itself copies each young object that may move the
//! first time it reaches it and rewrites the edge it came through, and every
//! later edge to the old copy ([`Core::mark_young`]): one pass over the live
//! heap, which visits every edge, so no barrier and no second (forwarding)
//! pass are needed. The heap's own lists get the new addresses as they are
//! marked, the CHARSXP cache the copied strings'; then the old copies are
//! retired. A stale `Sexp` to a young object reads poison at the next
//! collection; one to an older object, at the next full one.

use super::*;

/// The type byte of an old copy while a collection forwards edges: the new
/// address is in its link word (`layout::header::OFF_LINK`). Never seen outside a collection.
pub(super) const FORWARDED: u8 = 0xFE;

/// The type byte of a retired old copy (and, under gctorture, of a freed
/// slot): no SEXPTYPE has it, and the rest of the object is `0xDB` bytes,
/// so a stale `Sexp` reads garbage types and pointers and fails loudly.
pub const POISON_TYPE: u8 = 0xDB;

/// Under `ROBIGO_GC_MOVE=young:N`: a moving collection copied this object
/// (it is the copy), so the young collections leave it where it is. Fresh
/// objects have it clear. The header bit the boot heap's freeze uses as
/// [`flags::FROZEN`]: the moving modes never run with a freeze (the freeze
/// is off under them, and they cannot start after one), so the two never
/// meet. A copy made by any later code path that copies flags keeps it,
/// which only means the copy waits for a full collection to move.
pub(super) const OLD: u8 = flags::FROZEN;

/// `ROBIGO_GC_MOVE`: off (unset, empty or `0`), `Some(0)` for `1` (every
/// collection moves everything), `Some(n)` for `young:n` (every n-th moving
/// collection moves everything, the others the young objects).
///
/// # Panics
/// On any other value: a mistyped check must not silently not check.
pub(super) fn policy_from_env() -> Option<u32> {
    let v = std::env::var("ROBIGO_GC_MOVE").ok()?;
    match v.as_str() {
        "" | "0" => None,
        "1" => Some(0),
        _ => match v.strip_prefix("young:").and_then(|n| n.parse::<u32>().ok()) {
            Some(n) if n >= 1 => Some(n),
            _ => panic!("robigo: ROBIGO_GC_MOVE is 1 or young:N (N at least 1), not {v:?}"),
        },
    }
}

/// `ROBIGO_GC_MOVE=young:N`'s state.
pub(super) struct Young {
    /// Every `full_every`-th moving collection moves everything it may, the
    /// others only the young objects; 0 (`=1`): every one moves everything.
    full_every: u32,
    /// The moving collections so far, which pick the full ones.
    collections: u64,
    /// The collections that moved only the young objects.
    pub(super) young_only: u64,
    /// A young collection's old copies, retired after its marking.
    moved: Moved,
}

impl Young {
    pub(super) fn new(full_every: u32) -> Young {
        Young { full_every, collections: 0, young_only: 0, moved: Moved::default() }
    }
}

/// What [`Core::evacuate`] moved, for the forwarding and the retirement.
#[derive(Default)]
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
        // old copy keeps its new address in its link word until it is retired.
        unsafe {
            if x.typ() == FORWARDED {
                *e = Sexp(*(x.0.add(header::OFF_LINK) as *const *mut u8));
            }
        }
    }
}

/// A young collection's marking visitor: each edge is marked, and takes
/// the address of the object's copy when the marker moved it.
pub(super) struct MarkYoung<'a>(pub(super) &'a mut Core);

impl Visitor for MarkYoung<'_> {
    #[inline(always)]
    fn edge(&mut self, edge: &mut Sexp) {
        // SAFETY: as for `Mark`: a `MarkYoung` exists only inside a young
        // collection's marking (this module makes it), shown the edges of
        // the roots and of marked objects, each null, an old copy or live.
        unsafe {
            let q = self.0.mark_young(*edge);
            if q != *edge {
                *edge = q;
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

    /// Move objects at every collection from now on, or stop.
    /// `ROBIGO_GC_MOVE=1` sets it when the core is made.
    ///
    /// # Safety
    /// Stopping (`on` false) has no condition. Starting needs that C holds
    /// no address of an object handed to it while moving was off: what C
    /// has seen is recorded ([`crate::extra::SEEN_BY_C`]) only while moving
    /// is on, so such an object would move under C. Turn it on before
    /// anything is handed to C.
    pub unsafe fn set_gc_move(&mut self, on: bool) {
        // (after the boot boundary: its pinned freeze roots are not
        // forwarded, Sol step 2 r2 S6)
        assert!(
            !on || (!self.boundary_drawn() && self.frozen.is_empty()),
            "robigo: the moving collector cannot start after the boot heap's freeze"
        );
        if !on && self.gc_move {
            // OLD is the bit the boot heap's freeze uses as FROZEN: a heap
            // that stops moving keeps none of it (Sol r1 S3)
            self.for_each_live_object(|p| {
                // SAFETY: a live object's header is readable and writable.
                unsafe { *p.add(header::OFF_FLAGS) &= !OLD };
            });
        }
        self.gc_move = on;
        self.young.full_every = 0;
    }

    /// Move objects from now on as `ROBIGO_GC_MOVE=young:every` does: every
    /// `every`-th moving collection moves everything it may, the others only
    /// the young objects.
    ///
    /// # Safety
    /// As for [`Core::set_gc_move`] with `on` true.
    ///
    /// # Panics
    /// If `every` is 0.
    pub unsafe fn set_gc_move_young(&mut self, every: u32) {
        assert!(
            !self.boundary_drawn() && self.frozen.is_empty(),
            "robigo: the moving collector cannot start after the boot heap's freeze"
        );
        assert!(every >= 1, "robigo: a young moving policy needs a full collection every N >= 1");
        self.gc_move = true;
        self.young.full_every = every;
    }

    /// The collections that moved only the young objects
    /// (`ROBIGO_GC_MOVE=young:N`; the others moved everything).
    pub fn young_collections(&self) -> u64 {
        self.young.young_only
    }

    /// Whether the collection starting now moves only the young objects;
    /// counts the moving collections. Not while the pin report watches: its
    /// scan is the full collection's.
    pub(super) fn young_collection(&mut self) -> bool {
        // (before the moves are armed nothing moves, and no collection
        // counts towards young:N's full ones)
        if !self.gc_move || !self.moves_armed {
            return false;
        }
        let y = &mut *self.young;
        y.collections += 1;
        let young = y.full_every > 1 && self.diag.is_none() && !y.collections.is_multiple_of(y.full_every as u64);
        y.young_only += young as u64;
        young
    }

    /// A young collection's pins, before anything moves: what a C word may
    /// point at is recorded as seen by C ([`Core::pin`]), so the marker
    /// that copies young objects leaves it where it is. Marks nothing: the
    /// segments are scanned again, as pins, after the weak references.
    ///
    /// # Safety
    /// At a young collection's start, on the core's thread.
    pub(super) unsafe fn pin_segments(&mut self) {
        // SAFETY: a callback and a context of matching types, on the core's
        // thread (the caller's contract).
        unsafe { crate::unwind::robigo_heap_spill(Self::pin_segments_cb, self as *mut Core as *mut c_void) }
    }

    /// # Safety
    /// As for [`Core::scan_segments`].
    unsafe extern "C" fn pin_segments_cb(ctx: *mut c_void) {
        // SAFETY: as in `scan_segments`: the words are C's to read.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            let (min, max) = (c.heap.min_addr, c.heap.max_addr);
            let marker = 0usize;
            let sp = std::hint::black_box(&marker) as *const usize as usize;
            let mut seg = c.scan_chain;
            while !seg.is_null() {
                let s = &*seg;
                let regs = s.regs.as_ptr() as usize;
                for (lo, hi) in [(s.lo.max(sp), s.hi.min(c.stack_top)), (regs, regs + std::mem::size_of_val(&s.regs))] {
                    crate::unwind::robigo_heap_scan_words(lo, hi, min, max, Self::pin_hit, ctx);
                }
                seg = s.prev;
            }
        }
    }

    /// `pin_segments`' answer to a word: what it keeps is pinned.
    ///
    /// # Safety
    /// `ctx` is the collecting core.
    unsafe extern "C" fn pin_hit(ctx: *mut c_void, _at: usize, v: usize) {
        // SAFETY: the caller's contract; `scan_candidates` gives allocated
        // objects.
        unsafe {
            let c = &mut *(ctx as *mut Core);
            for p in c.scan_candidates(v).into_iter().flatten() {
                c.pin(Sexp(p));
            }
        }
    }

    /// Mark `p` in a young collection and return where it is now: an old
    /// copy gives its copy's address; a young object that may move is
    /// copied first (the copy marked and queued for tracing); anything else
    /// is marked where it is. The marker's fast path is `mark_obj`'s: a
    /// null, immortal or marked object returns at once, and only an object
    /// seen for the first time takes [`Core::mark_young_first`]'s call.
    ///
    /// # Safety
    /// As for [`Core::mark_obj`], during a young collection's marking, after
    /// [`Core::pin_segments`].
    #[inline(always)]
    pub(super) unsafe fn mark_young(&mut self, p: Sexp) -> Sexp {
        // SAFETY: the caller's contract: `p` is null, an old copy (no flags,
        // never marked) or an allocated object, whose header and mark are
        // readable.
        unsafe {
            if p.is_null() {
                return p;
            }
            let f = p.flags();
            if f & flags::IMMORTAL != 0 || self.is_marked(p) {
                return p;
            }
            // most first visits are to old objects: marked here, as
            // `mark_obj` marks; only a young object or an old copy takes the
            // call
            if f & OLD != 0 {
                self.set_mark(p);
                return p;
            }
            self.mark_young_first(p)
        }
    }

    /// Mark the unmarked, mortal object `p` and queue it for tracing:
    /// `mark_obj`'s second half.
    ///
    /// # Safety
    /// `p` is an allocated, mortal object of this heap, not yet marked, during
    /// a collection's marking.
    #[inline(always)]
    unsafe fn set_mark(&mut self, p: Sexp) {
        // SAFETY: as in `mark_obj`: its block or large chunk's header.
        unsafe {
            let b = (p.0 as usize & !(BLOCK - 1)) as *mut Block;
            if (*b).magic == 0x524f4249474f4241 {
                let g = (p.0 as usize - Block::data_ptr(b) as usize) / GRANULE;
                *(*b).marks.as_mut_ptr().add(g >> 6) |= 1u64 << (g & 63);
                (*b).marked = (*b).marked.wrapping_add(1);
            } else {
                *(b as *mut u64).add(1) = self.epoch;
            }
            self.mark_stack.push(p);
        }
    }

    /// [`Core::mark_young`] of a young object or an old copy, not yet
    /// marked by this collection.
    ///
    /// # Safety
    /// As for [`Core::mark_young`]; `p` is unmarked, mortal and not [`OLD`].
    #[inline(never)]
    unsafe fn mark_young_first(&mut self, p: Sexp) -> Sexp {
        // SAFETY: the caller's contract.
        unsafe {
            if p.typ() == FORWARDED {
                return Sexp(*(p.0.add(header::OFF_LINK) as *const *mut u8));
            }
            if !p.seen_by_c()
                && let Some(q) = self.copy_young(p)
            {
                return q;
            }
            self.set_mark(p);
            p
        }
    }

    /// Copy the young, unmarked object `p`: the copy is marked, [`OLD`] and
    /// queued for tracing, the old copy forwards to it and waits for
    /// [`Core::retire_young`]. None if the system refuses a large copy (it
    /// stays where it is: a debug collector degrades).
    ///
    /// # Safety
    /// `p` is an unmarked object of this heap that may move, during a young
    /// collection's marking.
    unsafe fn copy_young(&mut self, p: Sexp) -> Option<Sexp> {
        // SAFETY: the caller's contract: `p`'s block or large chunk has a
        // header; a small object's slot is its class's size, a large one's
        // payload its chunk's; both are copied into fresh memory of the size.
        unsafe {
            let b = (p.0 as usize & !(BLOCK - 1)) as *mut Block;
            let q = if (*b).magic == 0x524f4249474f4241 {
                let size = SIZES[(*b).class as usize];
                let q = self.heap.alloc_or_abort(size);
                std::ptr::copy_nonoverlapping(p.0, q, size);
                self.charge_copy(q);
                self.mark_copy(q);
                self.young.moved.small.push(p.0);
                q
            } else {
                let base = b as usize;
                let c = *self.heap.chunks.iter().find(|c| c.base == base)?;
                let size = c.end - base - DATA_OFF;
                let q = self.heap.try_alloc_large(size)?;
                std::ptr::copy_nonoverlapping(p.0, q, size);
                *((q as usize - DATA_OFF) as *mut u64).add(1) = self.epoch;
                self.young.moved.large.push(base);
                q
            };
            *q.add(header::OFF_FLAGS) |= OLD;
            forward_to(p.0, q);
            self.mark_stack.push(Sexp(q));
            self.stats.moved += 1;
            Some(Sexp(q))
        }
    }

    /// The CHARSXP cache's entries of the strings a young collection
    /// copied take their copies (the cache is pruned on the marks next).
    ///
    /// # Safety
    /// After a young collection's marking, before the prune.
    pub(super) unsafe fn forward_young_chars(&mut self) {
        let olds: Vec<*mut u8> = self
            .young
            .moved
            .small
            .iter()
            .copied()
            .chain(self.young.moved.large.iter().map(|&b| (b + DATA_OFF) as *mut u8))
            .collect();
        for p in olds {
            // SAFETY: an old copy keeps its copy's address in its link word
            // until it is retired; the copy is a live object.
            unsafe {
                let q = Sexp(*(p.add(header::OFF_LINK) as *const *mut u8));
                if q.typ() == CHARSXP && q.gp() & gp::charsxp::CACHED != 0 {
                    self.chars.replace(Sexp(p), q);
                }
            }
        }
    }

    /// Retire a young collection's old copies, as [`Core::retire`] does,
    /// for copies that were never marked.
    ///
    /// # Safety
    /// After a young collection's marking and the CHARSXP fix-up.
    pub(super) unsafe fn retire_young(&mut self) {
        let moved = std::mem::take(&mut self.young.moved);
        for p in moved.small {
            // SAFETY: as in `retire`; the old copy was never marked.
            unsafe {
                let b = (p as usize & !(BLOCK - 1)) as *mut Block;
                std::ptr::write_bytes(p, 0xDB, SIZES[(*b).class as usize]);
                *p.add(header::OFF_FLAGS) = 0;
            }
        }
        // SAFETY: the caller's contract.
        unsafe { self.retire_large(moved.large) };
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
                            let p = Block::data_ptr(b).add(g * GRANULE);
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
                let q = self.heap.alloc_or_abort(size);
                std::ptr::copy_nonoverlapping(p, q, size);
                *q.add(header::OFF_FLAGS) |= OLD;
                self.charge_copy(q);
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
                *q.add(header::OFF_FLAGS) |= OLD;
                *((q as usize - DATA_OFF) as *mut u64).add(1) = self.epoch;
                forward_to(p, q);
            }
            moved.large.push(base);
        }
        self.stats.moved += (moved.small.len() + moved.large.len()) as u64;
        moved
    }

    /// A small copy's vector cells, in its new block ([`Block::cells`]) and
    /// in the vector heap's use: the old copy's block loses them at the
    /// sweep that frees it (its survivors no longer include it), so the use
    /// is unchanged, as a copy is the object and not an allocation.
    ///
    /// # Safety
    /// `q` is a fresh small copy of a live object, its header and length
    /// words copied.
    unsafe fn charge_copy(&mut self, q: *mut u8) {
        // SAFETY: the caller's contract.
        unsafe {
            let cells = crate::heapsize::object_cells(Sexp(q));
            charge_block(q, cells);
            self.sizes.valloc += cells;
        }
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
            let g = (q as usize - Block::data_ptr(b) as usize) / GRANULE;
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
            for x in self.precious.iter_mut().chain(self.weak_refs.iter_mut()) {
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
        // through `dyn Visitor`: one instance of the embedder's root walk for
        // both moving modes (`collect_needing`)
        roots.trace::<dyn Visitor>(&mut Forward);
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
                *p.add(header::OFF_FLAGS) = 0;
                let g = (p as usize - Block::data_ptr(b) as usize) / GRANULE;
                (*b).marks[g >> 6] &= !(1u64 << (g & 63));
                (*b).marked = (*b).marked.wrapping_sub(1);
            }
        }
        // SAFETY: the caller's contract.
        unsafe { self.retire_large(moved.large) };
    }

    /// Take the old large chunks out of the heap, without access, until the
    /// next collection.
    ///
    /// # Safety
    /// As for [`Core::retire`]: nothing refers to their objects.
    unsafe fn retire_large(&mut self, large: Vec<usize>) {
        for base in large {
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
                if unsafe {
                    *(c.base as *const u64).add(1) == epoch || *p.add(header::OFF_FLAGS) & flags::IMMORTAL != 0
                } {
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
                            let p = Block::data_ptr(b).add(g * GRANULE);
                            if (*b).marks[w] & (1u64 << (g & 63)) != 0
                                || *p.add(header::OFF_FLAGS) & flags::IMMORTAL != 0
                            {
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
        *p.add(header::OFF_TYPE) = FORWARDED;
        *p.add(header::OFF_FLAGS) = 0;
        *(p.add(header::OFF_LINK) as *mut *mut u8) = q;
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
