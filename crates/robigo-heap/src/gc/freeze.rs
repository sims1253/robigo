//! The boot heap's freeze (docs/ledger/g3/2026-11-21-h-boot-freeze-bar.md
//! and its amendment, 2026-11-21-h-boot-freeze-check.md).
//!
//! **Off by default:** `ROBIGO_GC_FREEZE=1` turns it on
//! ([`Core::freeze_enabled`]), until the frozen-reachability pass lands and
//! the default flips. Off, none of it runs.
//!
//! **Lazy:** the embedder draws the boot boundary at the end of its boot
//! ([`Core::freeze_boundary`]): a flag, and the blocks in use marked, no
//! object visited, so a session that never collects pays nothing. The first
//! collection after it, before it marks ([`Core::freeze_at_boundary`]),
//! makes every boot object reachable from the roots' freeze roots
//! ([`Trace::trace_freeze`]) immortal, so that no later collection traces it
//! again. A boot object is one allocated before the boundary: a later one
//! (`options(x = <1e8 doubles>)` stored under base before that collection)
//! stays ordinary, so that replacing it frees it. An immortal object is
//! never marked (marking stops at it) and never swept (the sweep keeps it by
//! its flag, and a block whose objects are all marked or immortal skips its
//! bitmap walk). [`Core::freeze`] freezes at once (the heap's tests).
//!
//! **The invariant:** at every collection, every ordinary (not immortal)
//! object that a frozen object points to is reachable from a root. It holds
//! because:
//! - at the freeze, every frozen object with an ordinary child is logged in
//!   the remembered set, which is a root;
//! - after it, every store of a pointer into a frozen object passes the store
//!   barrier (`Sexp::store`, `store_uncounted`), which logs the object on its
//!   first store: `make_immortal` sets `BARRIER`;
//! - C's first writable `DATAPTR` of a frozen vector of objects logs it too
//!   ([`Core::remember`]).
//!
//! **What is not frozen:** weak references (their death and their keys'
//! are observable), the keys of every weak reference (a finalizer's or a
//! weak value's), objects C has seen (`SEEN_BY_C`: C may write their
//! payload without a barrier), and whatever the embedder's `exclude` says
//! (its global environment). The walk stops at each of them, and they
//! stay ordinary.
//!
//! **Unfreezing, batched:** an event that needs a frozen object to become
//! collectable (a weak reference keyed on it: `reg.finalizer`,
//! `R_MakeWeakRef`) queues it ([`Core::request_unfreeze`]). Nothing frees
//! it before a collection anyway. The next collection's first step is one
//! pass over the frozen set that logs every frozen object referring to a
//! queued one. It then makes the queued objects ordinary again, so one
//! pass serves the whole queue. The queued objects' frozen children stay
//! frozen, and their own references to frozen objects need nothing: a
//! frozen object is never freed.
//!
//! **Reference counts:** an immortal object's count is not maintained, so a
//! frozen object's is set to the maximum: it reads as shared, and a modify
//! of a value base holds copies it (premise 7 measured no perf row doing
//! one). An unfrozen object keeps that count: shared, sticky.
//!
//! The moving collector's modes (`ROBIGO_GC_MOVE`) leave the freeze off,
//! so that they still move the whole heap.
//!
//! INTERIM(H): **frozen garbage stays a root.** A logged frozen object
//! whose last reference is cut (a detached environment attached at the
//! freeze, its hash table logged) keeps its ordinary children as of its
//! death for the rest of the session, and they are never finalized
//! (tests/rt/memory/freeze-garbage-detach.R differs from GNU R). The owner
//! is the frozen-reachability pass, H's next PR
//! (docs/ledger/g3/2026-11-21-h-boot-freeze-check.md).

use super::*;
use std::collections::HashSet;

/// What a frozen object's count reads: shared (`MAYBE_SHARED`).
const FROZEN_REFCNT: u16 = u16::MAX;

/// The edges of one object, collected.
struct Edges<'a>(&'a mut Vec<Sexp>);

impl Visitor for Edges<'_> {
    fn edge(&mut self, edge: &mut Sexp) {
        if !edge.is_null() {
            self.0.push(*edge);
        }
    }
}

/// The boot boundary's state, between [`Core::freeze_boundary`] and the
/// lazy freeze.
pub(super) struct BootBoundary {
    /// The address of what the embedder leaves out (its global
    /// environment): compared, never read.
    exclude: usize,
    /// The large chunks' bases at the boundary (their objects are boot
    /// objects; a small one's block says it: [`Block::boot`]).
    large: HashSet<usize, BuildFx>,
    /// The boot blocks' free lists, set aside for the boundary.
    lists: Vec<(usize, usize)>,
}

/// A root set's freeze roots, as a root set (`Trace::trace_freeze`).
struct FreezeView<'a, R: ?Sized>(&'a mut R);

impl<R: Trace + ?Sized> Trace for FreezeView<'_, R> {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        self.0.trace_freeze(v);
    }
    fn trace_search_path<V: Visitor + ?Sized>(&mut self, v: &mut V) -> bool {
        self.0.trace_search_path(v)
    }
}

impl Core {
    /// Whether the freeze runs in this heap: `ROBIGO_GC_FREEZE=1` (off by
    /// default), and not under the moving collector's modes, which the heap
    /// selected when it was made or by its setters (both set `gc_move`).
    pub fn freeze_enabled(&self) -> bool {
        self.freeze_on && !self.gc_move
    }

    /// Turn the freeze on or off for this heap, as `ROBIGO_GC_FREEZE` does
    /// when it is made (the tests' switch). Its check's period is the
    /// environment's, read now when turning it on.
    ///
    /// # Panics
    /// Once the boot boundary is drawn or anything is frozen.
    pub fn set_freeze_enabled(&mut self, on: bool) {
        assert!(
            self.boot_boundary.is_none() && self.frozen.is_empty(),
            "robigo: the freeze is switched before the boot boundary"
        );
        if on && !self.freeze_on {
            self.freeze_check = freeze_check_from_env();
        }
        self.freeze_on = on;
    }

    /// The boot boundary (the lazy freeze): the end of the boot, where
    /// #959's eager freeze ran. Only a flag and the boot blocks marked
    /// ([`Heap::mark_boot_blocks`]: O(blocks), no object visited), so a
    /// session that never collects pays nothing more. The first collection
    /// after it freezes, before it marks: every object reachable from the
    /// roots' freeze roots ([`Trace::trace_freeze`]) that was allocated
    /// before this boundary, the kinds that are never frozen and `exclude`
    /// left out (an address compared, nothing read: any address may be
    /// given). Nothing when the freeze is off or has run.
    pub fn freeze_boundary(&mut self, exclude: usize) {
        if !self.freeze_enabled() || self.boot_boundary.is_some() || !self.frozen.is_empty() {
            return;
        }
        let (large, lists) = self.heap.mark_boot_blocks();
        self.boot_boundary = Some(Box::new(BootBoundary { exclude, large: large.into_iter().collect(), lists }));
    }

    /// The lazy freeze, at the start of the first collection after the
    /// boot boundary (`collect_needing`), before it marks: the boundary's
    /// rule, then the boundary ends (the boot blocks take allocations
    /// again, from this collection's sweep on). A root set without freeze
    /// roots freezes nothing, and the boundary ends all the same.
    ///
    /// # Safety
    /// As for [`Core::collect`]: every edge `roots` shows is null or live.
    pub(super) unsafe fn freeze_at_boundary<R: Trace + ?Sized>(&mut self, roots: &mut R) {
        let Some(b) = self.boot_boundary.take() else { return };
        let exclude = b.exclude;
        // SAFETY: the caller's contract; the freeze roots are a part of the roots.
        unsafe { self.freeze_from(&mut FreezeView(roots), |x| x.addr() == exclude, Some(&b.large)) };
        self.heap.end_boot_blocks(&b.lists);
    }

    /// Whether `x` was allocated before the boot boundary: its block is a
    /// boot block, or it is the object of a large chunk `large` lists.
    ///
    /// # Safety
    /// `x` is a live object of this heap, at the boundary.
    unsafe fn is_boot_object(&self, x: Sexp, large: &HashSet<usize, BuildFx>) -> bool {
        let b = self.heap.block_of(x.0);
        // SAFETY: the caller's contract: `x`'s block (or large chunk) header
        // is readable; a large chunk's first word is 0, not the magic.
        unsafe {
            if (*b).magic == 0x524f4249474f4241 {
                (*b).boot != 0
                    && ((x.addr() - Block::data_ptr(b) as usize) / crate::layout::GRANULE) < (*b).boot_bump as usize
            } else {
                large.contains(&(b as usize))
            }
        }
    }

    /// How many frozen-reachability passes ran.
    pub fn frozen_passes(&self) -> u64 {
        self.frozen_passes
    }

    /// How many objects the freeze made immortal and are still frozen.
    pub fn frozen_count(&self) -> usize {
        self.frozen.len()
    }

    /// The bytes the frozen objects take: their slots' class sizes, and a
    /// large one's chunk (`ROBIGO_GC_STATS`'s report; premise 6's bound).
    pub fn frozen_bytes(&self) -> usize {
        let large: HashMap<usize, usize, BuildFx> =
            self.heap.chunks.iter().filter(|c| c.large).map(|c| (c.base, c.end - c.base)).collect();
        self.frozen
            .iter()
            .map(|f| {
                let b = self.heap.block_of(f.0);
                // SAFETY: a frozen object is live: its block (or large
                // chunk) header is readable.
                large.get(&(b as usize)).copied().unwrap_or_else(|| unsafe { SIZES[(*b).class as usize] })
            })
            .sum()
    }

    /// Freeze every object reachable from `roots` (see the module's doc),
    /// stopping at the objects `exclude` names and at the kinds that are
    /// never frozen. Returns how many objects are frozen when it ends: those
    /// it froze less those the thaw at its end made ordinary again (the
    /// weak keys' and external data's frozen referrers). Run once, after a
    /// full collection, so that only survivors are frozen. Nothing under
    /// the moving collector's modes.
    ///
    /// # Safety
    /// Every edge `roots` shows is null or a live object of this heap, as
    /// for [`Core::collect`]; nothing else runs on the heap meanwhile.
    pub unsafe fn freeze<R: Trace + ?Sized>(&mut self, roots: &mut R, exclude: impl Fn(Sexp) -> bool) -> usize {
        // SAFETY: the caller's contract.
        unsafe { self.freeze_from(roots, exclude, None) }
    }

    /// [`Core::freeze`], and with `boot` the lazy freeze's rule: only the
    /// objects allocated before the boot boundary (the others stay
    /// ordinary, and a frozen object that points to one is logged).
    ///
    /// # Safety
    /// As for [`Core::freeze`].
    unsafe fn freeze_from<R: Trace + ?Sized>(
        &mut self,
        roots: &mut R,
        exclude: impl Fn(Sexp) -> bool,
        boot: Option<&HashSet<usize, BuildFx>>,
    ) -> usize {
        if !self.freeze_enabled() || !self.frozen.is_empty() {
            return 0;
        }
        let mut work = Vec::new();
        roots.trace(&mut Edges(&mut work));
        // the keys of weak references: their death is observable
        let mut keys: HashSet<usize, BuildFx> = HashSet::default();
        for &w in &self.weak_refs {
            // SAFETY: a registered weak reference: element 0 is its key.
            keys.insert(unsafe { Self::weak_field(w, 0) }.addr());
        }
        let mut stopped: HashSet<usize, BuildFx> = HashSet::default();
        let mut frozen = Vec::new();
        let mut kids = Vec::new();
        while let Some(x) = work.pop() {
            // SAFETY: the caller's contract: `x` is null or live.
            unsafe {
                if x.is_null() || x.flags() & flags::IMMORTAL != 0 || stopped.contains(&x.addr()) {
                    continue;
                }
                let observable = keys.contains(&x.addr()) || x.flags() & flags::EXTDATA != 0;
                let never = observable
                    || x.typ() == WEAKREFSXP
                    || x.extra() & crate::extra::SEEN_BY_C != 0
                    // replaced soon after the boot (C1's translation-cache
                    // stub): left ordinary, so its replacement overwrites no
                    // frozen value (the bit means this on a RAWSXP only)
                    || (x.typ() == RAWSXP && x.extra() & crate::extra::vector::NO_FREEZE != 0)
                    || exclude(x)
                    || boot.is_some_and(|l| !self.is_boot_object(x, l));
                if never {
                    stopped.insert(x.addr());
                    continue;
                }
                self.make_immortal_raw(x);
                x.set_flag_raw(flags::FROZEN, true);
                if x.typ() != CHARSXP {
                    x.set_refcnt(FROZEN_REFCNT);
                }
                frozen.push(x);
                kids.clear();
                x.trace_fields(&mut Edges(&mut kids));
                work.extend_from_slice(&kids);
            }
        }
        // (unsorted: membership is the header's FROZEN bit)
        self.frozen = frozen;
        // a frozen object with an ordinary child is a root from now on (the
        // global environment's referrers, a weak reference's, C's objects')
        for i in 0..self.frozen.len() {
            let f = self.frozen[i];
            // SAFETY: frozen objects are live; their fields are null or live.
            unsafe {
                if self.has_ordinary_child(f, &mut kids) {
                    self.remember(f);
                }
            }
        }
        // A weak key or an external-data vector stays ordinary (the walk
        // stopped at it), and a frozen referrer of one is logged above. Its
        // reachability decides its death (Sol S3) because a referrer that
        // becomes garbage is thawed by the next frozen-reachability pass:
        // the edge that made it garbage was a dirty event.
        // SAFETY: the caller's contract.
        self.path_snapshot = unsafe { self.search_path_frozen(roots) };
        // (notes made before the freeze name no frozen object)
        self.remembered.1.set(0);
        self.frozen.len()
    }

    /// Whether `f` points to an object that is not immortal.
    ///
    /// # Safety
    /// `f` is a live object; its fields are null or live.
    unsafe fn has_ordinary_child(&self, f: Sexp, kids: &mut Vec<Sexp>) -> bool {
        kids.clear();
        // SAFETY: the caller's contract.
        unsafe {
            f.trace_fields(&mut Edges(kids));
            kids.iter().any(|k| k.flags() & flags::IMMORTAL == 0)
        }
    }

    /// Log `x` in the remembered set (a root at every collection), once: the
    /// store barrier's slow path without a store. For an immortal object
    /// whose fields may change without a barrier (C's writable `DATAPTR` of
    /// a frozen vector of objects). Nothing for an object already logged
    /// or not immortal.
    ///
    /// # Safety
    /// `x` is a live object of this heap, on its thread, outside a
    /// collection.
    pub unsafe fn remember(&mut self, x: Sexp) {
        // SAFETY: the caller's contract (`barrier_only`'s).
        unsafe {
            if x.flags() & (flags::IMMORTAL | flags::BARRIER) == flags::IMMORTAL | flags::BARRIER
                && !crate::shared::is_shared(x)
            {
                super::log_once(x);
            }
        }
    }

    /// C's first writable `DATAPTR` of a frozen vector of objects (api.c):
    /// it is logged now, so that C's raw writes into its elements are traced
    /// until the next collection, and queued to become ordinary there, so
    /// that the frozen graph no longer follows edges C may rewrite without
    /// a barrier (the pass's bar, table (b)). Nothing for another object.
    ///
    /// # Safety
    /// As for [`Core::remember`].
    pub unsafe fn exposed_to_c(&mut self, x: Sexp) {
        // SAFETY: the caller's contract.
        unsafe {
            self.remember(x);
            if self.is_frozen(x) {
                // C has its payload: once ordinary it must not move (Sol r1 S2)
                x.set_extra(x.extra() | crate::extra::SEEN_BY_C);
                self.pending_unfreeze.push(x);
            }
        }
    }

    /// Whether `x` is one of the freeze's objects: its [`flags::FROZEN`]
    /// bit (the frozen list is not searched, nor kept sorted for it).
    ///
    /// # Safety
    /// `x` is a live object of this heap.
    pub unsafe fn is_frozen(&self, x: Sexp) -> bool {
        // SAFETY: the caller's contract.
        unsafe { x.flags() & flags::FROZEN != 0 }
    }

    /// Queue a frozen object to become ordinary at the next collection (a
    /// weak reference keyed on it is being made). Nothing for any other
    /// object.
    ///
    /// # Safety
    /// `x` is null or a live object of this heap (`register_weak_ref`'s key).
    pub(super) unsafe fn request_unfreeze(&mut self, x: Sexp) {
        // an ordinary key pays one header test (Sol S7)
        // SAFETY: the caller's contract.
        if x.is_null() || unsafe { x.flags() } & flags::IMMORTAL == 0 {
            return;
        }
        // SAFETY: the caller's contract: `x` is live.
        if unsafe { self.is_frozen(x) } {
            self.pending_unfreeze.push(x);
        }
    }

    /// The frozen environments on the search path that `roots` shows
    /// ([`Trace::trace_search_path`]), by address: the deferred search-path
    /// test's snapshot.
    ///
    /// # Safety
    /// Every edge `roots` shows is null or live.
    unsafe fn search_path_frozen<R: Trace + ?Sized>(&self, roots: &mut R) -> Option<Vec<usize>> {
        let mut envs = Vec::new();
        if !roots.trace_search_path(&mut Edges(&mut envs)) {
            // no search path to compare with: unknown (Sol r3 S11)
            return None;
        }
        // SAFETY: the caller's contract.
        Some(envs.into_iter().filter(|e| unsafe { self.is_frozen(*e) }).map(|e| e.addr()).collect())
    }

    /// The frozen-reachability pass (docs/ledger/g3/ the pass's bar), at a
    /// collection's start, before marking: when an edge into a frozen
    /// object left the frozen graph since the last pass (the heap's
    /// [`Remembered::DIRTY`] note, the search path's frozen members
    /// changed, or an object queued to unfreeze), one walk from the roots'
    /// freeze roots through frozen objects decides what stays frozen:
    /// - the queued objects become ordinary first, so the walk does not
    ///   follow them;
    /// - every frozen object the walk does not reach becomes ordinary (its
    ///   count stays at the frozen maximum) and leaves the log;
    /// - every frozen object it reaches that points to an ordinary one is
    ///   logged (a root), as at the freeze.
    ///
    /// Marking then runs as before: an object made ordinary that is still
    /// reachable is traced from now on, and one that is not is swept.
    ///
    /// # Safety
    /// As for [`Core::collect`]: every edge `roots` shows is null or live;
    /// outside marking.
    pub(super) unsafe fn frozen_pass<R: Trace + ?Sized>(&mut self, roots: &mut R) {
        let notes = self.remembered.1.get();
        if self.frozen.is_empty() {
            self.pending_unfreeze.clear();
            self.remembered.1.set(0);
            return;
        }
        // (a root set without freeze roots, or without the search path,
        // cannot decide: it leaves every note, the snapshot and the queue to
        // a collection that can: Sol r2 S5)
        let mut dirty = notes & Remembered::DIRTY != 0 || !self.pending_unfreeze.is_empty();
        let mut snapshot = None;
        if notes & Remembered::PATH_WRITTEN != 0 {
            let mut envs = Vec::new();
            if !roots.trace_search_path(&mut Edges(&mut envs)) {
                return;
            }
            // SAFETY: the caller's contract.
            let now: Vec<usize> =
                envs.into_iter().filter(|e| unsafe { self.is_frozen(*e) }).map(|e| e.addr()).collect();
            // (no snapshot known: any write of the path counts, Sol r3 S11)
            dirty |= self.path_snapshot.as_ref().is_none_or(|old| old.iter().any(|a| !now.contains(a)));
            snapshot = Some(now);
        }
        if !dirty {
            if let Some(now) = snapshot {
                self.path_snapshot = Some(now);
            }
            self.remembered.1.set(notes & !Remembered::PATH_WRITTEN);
            return;
        }
        // SAFETY: the caller's contract: frozen objects and every edge the
        // roots show are live; outside marking, the log is not borrowed.
        unsafe {
            // The walk's reached set is the mark bitmap: a small object's
            // bit, set without its block's count (`is_marked` answers true
            // for an immortal object without reading it, the sweep clears
            // every bitmap, and an object thawed below was never reached,
            // so it carries no bit into marking); a large object's in a set.
            let mut large: HashSet<usize, BuildFx> = HashSet::default();
            let mut work = Vec::new();
            if !roots.trace_freeze(&mut Edges(&mut work)) {
                // a root set that shows no freeze roots cannot say what the
                // frozen graph reaches: nothing is thawed, and nothing is
                // consumed (Sol r1 S5, r2 S5)
                return;
            }
            self.remembered.1.set(0);
            self.frozen_passes += 1;
            for x in std::mem::take(&mut self.pending_unfreeze) {
                if self.is_frozen(x) {
                    self.make_mortal(x);
                }
            }
            // the walk does not follow a queued object: it was made ordinary
            // above, so the roots' edges to it are ordinary ones
            let mut kept = Vec::with_capacity(self.frozen.len());
            let mut kids = Vec::new();
            while let Some(x) = work.pop() {
                if x.is_null() || !self.is_frozen(x) || !self.reach(x, &mut large) {
                    continue;
                }
                kept.push(x);
                kids.clear();
                x.trace_fields(&mut Edges(&mut kids));
                // a reached object's frozen children are reached too, so
                // whether it points to an ordinary object is known here
                let mut ordinary = false;
                for &k in &kids {
                    if k.flags() & flags::FROZEN != 0 {
                        work.push(k);
                    } else if k.flags() & flags::IMMORTAL == 0 {
                        ordinary = true;
                    }
                }
                if ordinary {
                    self.remember(x);
                }
            }
            for f in std::mem::take(&mut self.frozen) {
                if self.is_frozen(f) && !self.reached(f, &large) {
                    self.make_mortal(f);
                }
            }
            // the walk's bits go: the pass leaves the bitmaps as it found
            // them, whether or not a sweep follows
            for &f in &kept {
                self.unreach(f);
            }
            self.frozen = kept;
            // an object made ordinary leaves the log (it is traced if it is
            // reachable, and must die if it is not); the other immortal
            // objects' entries stay
            (*self.remembered.0.get()).retain(|l| l.flags() & flags::IMMORTAL != 0);
            // (a root set without the search path keeps the old snapshot: a
            // stale one can only call a later write a loss, Sol r3 S11)
            if let Some(now) = self.search_path_frozen(roots) {
                self.path_snapshot = Some(now);
            }
        }
    }

    /// `ROBIGO_GC_FREEZE_CHECK`'s I2 and I3 (the pass's bar, (d)), at a
    /// checked collection after the pass step:
    /// - **I2:** every frozen object is reachable from the roots' freeze
    ///   roots through frozen objects. A pass just ran, or no dirty event
    ///   since the last one removed a frozen edge: so an unreached frozen
    ///   object means an edge left the frozen graph without a note (a
    ///   missing row in the bar's table (b)).
    /// - **I3:** [`flags::FROZEN`] is set on exactly the frozen list's
    ///   members, each of them `IMMORTAL`.
    ///
    /// A failure is reported as the check's others are (`check_failed`).
    ///
    /// # Safety
    /// At a collection's start, after the pass step: every edge `roots`
    /// shows is null or live.
    pub(super) unsafe fn check_frozen_reach<R: Trace + ?Sized>(&self, roots: &mut R) {
        let mut work = Vec::new();
        // I2 holds only once the pass step has decided: a pass that deferred
        // (its roots could not decide) leaves its notes and queue for a
        // later collection, and a frozen object may be unreachable meanwhile
        // (Sol r3 S10)
        let decided = self.remembered.1.get() & (Remembered::DIRTY | Remembered::PATH_WRITTEN) == 0
            && self.pending_unfreeze.is_empty();
        let with_roots = roots.trace_freeze(&mut Edges(&mut work)) && decided;
        let mut reached: HashSet<usize, BuildFx> = HashSet::default();
        let mut kids = Vec::new();
        // SAFETY: the caller's contract; frozen objects are live.
        unsafe {
            while let Some(x) = work.pop() {
                if x.is_null() || !self.is_frozen(x) || !reached.insert(x.addr()) {
                    continue;
                }
                kids.clear();
                x.trace_fields(&mut Edges(&mut kids));
                work.extend_from_slice(&kids);
            }
            let mut listed: HashSet<usize, BuildFx> = HashSet::default();
            for &f in &self.frozen {
                if f.flags() & flags::IMMORTAL == 0 {
                    check_failed(format!(
                        "robigo: ROBIGO_GC_FREEZE_CHECK: {:#x} (type {}) is on the frozen list but not IMMORTAL",
                        f.addr(),
                        f.typ()
                    ));
                }
                // (a pinned object, IMMORTAL without FROZEN, waits on the list
                // for the next pass to drop it: Sol r1 S1)
                if f.flags() & flags::FROZEN == 0 {
                    continue;
                }
                if !listed.insert(f.addr()) {
                    check_failed(format!(
                        "robigo: ROBIGO_GC_FREEZE_CHECK: {:#x} (type {}) is on the frozen list twice",
                        f.addr(),
                        f.typ()
                    ));
                }
                if with_roots && !reached.contains(&f.addr()) {
                    check_failed(format!(
                        "robigo: ROBIGO_GC_FREEZE_CHECK: frozen {:#x} (type {}) is unreachable from the freeze roots, and no dirty event was noted since the last frozen-reachability pass",
                        f.addr(),
                        f.typ()
                    ));
                }
            }
            let mut stray = None;
            self.for_each_live_object(|p| {
                if stray.is_none() && *p.add(header::OFF_FLAGS) & flags::FROZEN != 0 && !listed.contains(&(p as usize))
                {
                    stray = Some(p as usize);
                }
            });
            if let Some(p) = stray {
                check_failed(format!(
                    "robigo: ROBIGO_GC_FREEZE_CHECK: {p:#x} has FROZEN but is not on the frozen list"
                ));
            }
        }
    }

    /// Mark frozen `x` reached by the pass's walk; false if it was already.
    /// A small object's mark bit (not its block's count: see
    /// [`Core::frozen_pass`]), a large one's base in `large`.
    ///
    /// # Safety
    /// `x` is a live frozen object, before this collection's marking.
    unsafe fn reach(&self, x: Sexp, large: &mut HashSet<usize, BuildFx>) -> bool {
        let b = self.heap.block_of(x.0);
        // SAFETY: the caller's contract: `x`'s block (or chunk) is readable,
        // and a granule index from a live object's address is in range.
        unsafe {
            if (*b).magic == 0x524f4249474f4241 {
                let g = (x.0 as usize - Block::data_ptr(b) as usize) / GRANULE;
                let (w, bit) = (g >> 6, 1u64 << (g & 63));
                let was = (*b).marks[w] & bit != 0;
                (*b).marks[w] |= bit;
                !was
            } else {
                large.insert(b as usize)
            }
        }
    }

    /// Clear the walk's mark bit of a small reached object ([`Core::reach`]).
    ///
    /// # Safety
    /// As for [`Core::reach`].
    unsafe fn unreach(&self, x: Sexp) {
        let b = self.heap.block_of(x.0);
        // SAFETY: as for `reach`.
        unsafe {
            if (*b).magic == 0x524f4249474f4241 {
                let g = (x.0 as usize - Block::data_ptr(b) as usize) / GRANULE;
                (*b).marks[g >> 6] &= !(1u64 << (g & 63));
            }
        }
    }

    /// Whether frozen `x` was reached by the pass's walk ([`Core::reach`]).
    ///
    /// # Safety
    /// As for [`Core::reach`].
    unsafe fn reached(&self, x: Sexp, large: &HashSet<usize, BuildFx>) -> bool {
        let b = self.heap.block_of(x.0);
        // SAFETY: as for `reach`.
        unsafe {
            if (*b).magic == 0x524f4249474f4241 {
                let g = (x.0 as usize - Block::data_ptr(b) as usize) / GRANULE;
                (*b).marks[g >> 6] & (1u64 << (g & 63)) != 0
            } else {
                large.contains(&(b as usize))
            }
        }
    }

    /// Undo [`Core::make_immortal`] for a frozen object: an ordinary object
    /// again. Its reference count stays at the frozen maximum (shared).
    /// The caller takes it out of the frozen set and the log.
    ///
    /// # Safety
    /// `x` is a live frozen object, outside marking.
    unsafe fn make_mortal(&mut self, x: Sexp) {
        // SAFETY: the caller's contract.
        unsafe {
            x.set_flag_raw(flags::IMMORTAL | flags::BARRIER | flags::FROZEN, false);
            if x.typ() == CHARSXP && x.gp() & gp::charsxp::CACHED != 0 {
                self.immortal_chars -= 1;
            }
            let b = self.heap.block_of(x.0);
            if (*b).magic == 0x524f4249474f4241 {
                (*b).immortals -= 1;
            }
        }
    }

    /// The invariant outside a collection: every frozen object that points
    /// to an ordinary one is in the remembered set (checked against the log
    /// itself, not the `BARRIER` bit). Panics, naming both, otherwise.
    ///
    /// # Safety
    /// Outside marking: frozen objects and their fields are live.
    pub unsafe fn check_frozen(&self) {
        // SAFETY: outside marking, the log is not borrowed elsewhere.
        let log: HashSet<usize, BuildFx> = unsafe { &*self.remembered.0.get() }.iter().map(|r| r.addr()).collect();
        let mut kids = Vec::new();
        for &f in &self.frozen {
            if log.contains(&f.addr()) {
                continue;
            }
            kids.clear();
            // SAFETY: the caller's contract.
            unsafe {
                f.trace_fields(&mut Edges(&mut kids));
                if let Some(k) = kids.iter().find(|k| k.flags() & flags::IMMORTAL == 0) {
                    frozen_panic(f, *k, "and is not in the remembered set");
                }
            }
        }
    }

    /// `ROBIGO_GC_FREEZE_CHECK`'s independent trace (the bar's (c)), before
    /// marking: every ordinary object reachable from the precise roots (the
    /// embedder's, the precious list, the weak references, the remembered
    /// log) when frozen objects are walked as if ordinary. The production
    /// trace's marks are compared with it after the strong trace
    /// ([`Core::check_frozen_marked`]).
    ///
    /// # Safety
    /// At a collection's start: every edge `roots` shows and every object
    /// reached is null or live.
    pub(super) unsafe fn independent_reach<R: Trace + ?Sized>(&self, roots: &mut R) -> Vec<Sexp> {
        let mut work = Vec::new();
        roots.trace(&mut Edges(&mut work));
        work.extend_from_slice(&self.precious);
        work.extend_from_slice(&self.weak_refs);
        // SAFETY: outside marking, the log is not borrowed elsewhere.
        let log = unsafe { &*self.remembered.0.get() };
        let mut kids = Vec::new();
        for &l in log {
            kids.clear();
            // SAFETY: the log's objects are live.
            unsafe { l.trace_fields(&mut Edges(&mut kids)) };
            work.extend_from_slice(&kids);
        }
        let mut seen: HashSet<usize, BuildFx> = HashSet::default();
        let mut ordinary = Vec::new();
        while let Some(x) = work.pop() {
            if x.is_null() || !seen.insert(x.addr()) {
                continue;
            }
            // SAFETY: the caller's contract.
            unsafe {
                let immortal = x.flags() & flags::IMMORTAL != 0;
                if immortal && !self.is_frozen(x) {
                    // another immortal object (a constant, a PRINTNAME): not
                    // traced by either trace; its logged fields are above
                    continue;
                }
                if !immortal {
                    ordinary.push(x);
                }
                kids.clear();
                x.trace_fields(&mut Edges(&mut kids));
                work.extend_from_slice(&kids);
            }
        }
        ordinary
    }

    /// `ROBIGO_GC_FREEZE_CHECK`, at a checked collection after marking: every
    /// ordinary object a frozen object points to is marked, which is what
    /// the invariant must give the collector (Sol S5: the marks, not the
    /// log's bookkeeping). Panics, naming both, otherwise.
    ///
    /// # Safety
    /// During a collection, after marking and before the sweep.
    pub(super) unsafe fn check_frozen_marked(&self) {
        let mut kids = Vec::new();
        for &f in &self.frozen {
            kids.clear();
            // SAFETY: the caller's contract.
            unsafe {
                f.trace_fields(&mut Edges(&mut kids));
                if let Some(k) = kids.iter().find(|k| k.flags() & flags::IMMORTAL == 0 && !self.is_marked(**k)) {
                    frozen_panic(f, *k, "unmarked after marking");
                }
            }
        }
        // then the independent trace's objects are all marked (the bar's
        // (c)); the direct check above names the frozen referrer first (r4 S5)
        for &x in &self.freeze_reach {
            // SAFETY: the caller's contract; reached at this collection's start.
            unsafe {
                if x.flags() & flags::IMMORTAL == 0 && !self.is_marked(x) {
                    check_failed(format!(
                        "robigo: ROBIGO_GC_FREEZE_CHECK: {:#x} (type {}) is reachable but the frozen-aware trace left it unmarked",
                        x.addr(),
                        x.typ()
                    ));
                }
            }
        }
    }
}

#[cold]
fn frozen_panic(f: Sexp, k: Sexp, what: &str) -> ! {
    // SAFETY: both live (the callers' contracts).
    let (ft, kt) = unsafe { (f.typ(), k.typ()) };
    check_failed(format!(
        "robigo: ROBIGO_GC_FREEZE_CHECK: frozen {:#x} (type {ft}) points to the ordinary {:#x} (type {kt}) {what}",
        f.addr(),
        k.addr()
    ))
}

/// A failed check: the message appended to `ROBIGO_GC_FREEZE_CHECK_REPORT`
/// when it names a file (scripts/rt-ci.py fails a step whose report has a
/// line, whatever the case's own output: Sol r7 S14), then a panic.
fn check_failed(msg: String) -> ! {
    if let Some(path) = std::env::var_os("ROBIGO_GC_FREEZE_CHECK_REPORT").filter(|p| !p.is_empty()) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{msg}");
        }
    }
    panic!("{msg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct One(Sexp);
    impl Trace for One {
        fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
            v.edge(&mut self.0);
        }
    }

    /// A root set whose freeze roots are a part of it, as the embedder's.
    struct WithFreeze {
        all: Vec<Sexp>,
        freeze: Sexp,
    }
    impl Trace for WithFreeze {
        fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
            for x in &mut self.all {
                v.edge(x);
            }
        }
        fn trace_freeze<V: Visitor + ?Sized>(&mut self, v: &mut V) -> bool {
            v.edge(&mut self.freeze);
            true
        }
    }

    /// The freeze is off unless `ROBIGO_GC_FREEZE=1` says otherwise: off,
    /// the boundary marks nothing and no collection freezes.
    #[test]
    fn the_freeze_is_off_by_default() {
        let mut c = Core::new();
        if std::env::var_os("ROBIGO_GC_FREEZE").is_some() {
            c.set_freeze_enabled(false);
        }
        assert!(!c.freeze_enabled());
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let x = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            c.freeze_boundary(nil.addr());
            assert!(c.boot_boundary.is_none(), "no boundary drawn");
            assert_eq!((*c.heap.block_of(x.0)).boot, 0, "no block marked");
        }
    }

    /// Under Miri as well (no collection): the boot boundary marks the
    /// blocks in use, an object allocated after it lands in a block that is
    /// not one, and ending it makes the boot blocks ordinary again.
    #[test]
    fn the_boundary_puts_later_objects_outside_the_boot_blocks() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let before = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            let big = c.alloc_vector(REALSXP, 100_000).unwrap();
            c.freeze_boundary(nil.addr());
            let b = c.boot_boundary.take().unwrap();
            let after = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            let big_after = c.alloc_vector(REALSXP, 100_000).unwrap();
            assert!(c.is_boot_object(before, &b.large) && c.is_boot_object(big, &b.large));
            assert!(!c.is_boot_object(after, &b.large), "a later object is carved above the boundary's mark");
            assert!(
                !c.is_boot_object(big_after, &b.large),
                "and a later large one in a chunk the boundary did not list"
            );
            c.heap.end_boot_blocks(&b.lists);
            assert_eq!((*c.heap.block_of(before.0)).boot, 0, "the boundary ended");
        }
    }

    /// The lazy freeze: nothing is frozen at the boundary; the next
    /// collection freezes what the freeze roots reach of the boot objects,
    /// and a later object they reach stays ordinary, its frozen referrer
    /// logged (the boundary's `options(x = <1e8 doubles>)` case).
    #[test]
    #[cfg(not(miri))]
    fn the_first_collection_after_the_boundary_freezes_the_boot_objects_only() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap, rooted by `roots`.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 3).unwrap();
            let cell = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, cell, nil, nil).unwrap();
            c.freeze_boundary(nil.addr());
            assert_eq!(c.frozen_count(), 0, "nothing frozen at the boundary");
            // the session stores a later object under the freeze roots
            let later = c.alloc_vector(REALSXP, 1_000).unwrap();
            head.store(
                head.as_ptr().add(crate::layout::node::OFF_CDR) as *mut Sexp,
                c.alloc_node(LISTSXP, later, nil, nil).unwrap(),
            );
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.collect(&mut roots);
            assert!(c.is_frozen(head) && c.is_frozen(cell) && c.is_frozen(leaf), "the boot objects froze");
            let spine = head.cdr();
            assert!(!c.is_frozen(spine) && !c.is_frozen(later), "the later ones did not");
            assert_eq!(head.flags() & flags::BARRIER, 0, "their frozen referrer is logged");
            assert!(c.boot_boundary.is_none(), "the boundary ended");
            c.check_frozen();
            // the later objects die when the session drops them
            head.store(head.as_ptr().add(crate::layout::node::OFF_CDR) as *mut Sexp, nil);
            c.collect(&mut roots);
            c.check_frozen();
            assert!(c.is_frozen(head));
        }
    }

    /// Under Miri as well (no collection): what a root reaches is frozen,
    /// an excluded object is not, the invariant holds, and a store of an
    /// ordinary object into a frozen one logs it.
    #[test]
    fn a_frozen_graph_keeps_the_invariant_through_a_store() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 3).unwrap();
            let keep = c.alloc_vector(REALSXP, 1).unwrap();
            let cell = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, keep, cell, nil).unwrap();
            let n = c.freeze(&mut One(head), |x| x == keep);
            assert_eq!(n, 3, "head, cell and leaf");
            assert!(c.is_frozen(head) && c.is_frozen(cell) && c.is_frozen(leaf));
            assert!(!c.is_frozen(keep) && keep.flags() & flags::IMMORTAL == 0, "excluded");
            // head points to the excluded `keep`: logged at the freeze
            assert_eq!(head.flags() & flags::BARRIER, 0, "a frozen object with an ordinary child is logged");
            assert_ne!(cell.flags() & flags::BARRIER, 0, "one with frozen children only is not");
            assert_eq!(head.refcnt(), FROZEN_REFCNT, "a frozen object reads as shared");
            c.check_frozen();
            // a store of an ordinary object into a frozen one passes the barrier
            let young = c.alloc_vector(INTSXP, 1).unwrap();
            cell.store(cell.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, young);
            assert_eq!(cell.flags() & flags::BARRIER, 0, "the store logged it");
            c.check_frozen();
            // a second freeze does nothing
            assert_eq!(c.freeze(&mut One(young), |_| false), 0);
        }
    }

    /// Cut `head`'s CAR (a frozen cell's), through the barrier.
    ///
    /// # Safety
    /// `head` is a live cell; `nil` is R_NilValue.
    unsafe fn cut_car(head: Sexp, nil: Sexp) {
        // SAFETY: the caller's contract.
        unsafe { head.store(head.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, nil) }
    }

    /// A weak reference made after the freeze on a key that a frozen
    /// container reaches through an ordinary object (Sol r2 S3): the pass
    /// thaws the key; the container stays frozen and logged while it is
    /// reachable (the key is then reachable in GNU R too), and once its last
    /// edge is cut the next pass thaws it and drops it from the log, so only
    /// reachability decides.
    #[test]
    fn a_weak_key_after_the_freeze_thaws_and_its_container_thaws_when_cut_off() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let key = c.alloc_node(ENVSXP, nil, nil, nil).unwrap();
            let mid = c.alloc_node(LISTSXP, key, nil, nil).unwrap();
            let container = c.alloc_node(LISTSXP, mid, nil, nil).unwrap();
            let rest = c.alloc_node(LISTSXP, key, nil, nil).unwrap();
            let root = c.alloc_node(LISTSXP, container, rest, nil).unwrap();
            let mut roots = WithFreeze { all: vec![root], freeze: root };
            c.freeze(&mut roots, |x| x == mid);
            assert!(c.is_frozen(key) && c.is_frozen(container) && !c.is_frozen(mid));
            assert_eq!(container.flags() & flags::BARRIER, 0, "the container reaches the ordinary `mid`: logged");
            let w = c.alloc_vector(WEAKREFSXP, 4).unwrap();
            w.set_elt(0, key);
            w.set_elt(1, nil);
            w.set_elt(2, nil);
            c.register_weak_ref(w);
            c.frozen_pass(&mut roots);
            assert!(!c.is_frozen(key) && key.flags() & flags::IMMORTAL == 0, "the key thawed");
            assert!(c.is_frozen(container) && c.is_frozen(rest), "its referrers stay frozen");
            assert_eq!(rest.flags() & flags::BARRIER, 0, "and the one that now points to an ordinary key is logged");
            c.check_frozen();
            // the container's last edge cut: dirty, and the next pass thaws it
            cut_car(root, nil);
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "a frozen value overwritten in a frozen object");
            c.frozen_pass(&mut roots);
            assert!(
                !c.is_frozen(container) && container.flags() & flags::IMMORTAL == 0,
                "the cut-off container thawed"
            );
            assert!(!(*c.remembered.0.get()).contains(&container), "and left the log");
            assert!(c.is_frozen(root) && c.is_frozen(rest), "what the roots still reach stays frozen");
            c.check_frozen();
        }
    }

    /// A weak reference registered before the freeze on a key that a frozen
    /// container reaches through an excluded ordinary object (Sol r3 S3):
    /// the key stays ordinary and the container is logged; cut off, the
    /// container thaws at the next pass.
    #[test]
    fn an_earlier_weak_key_stays_ordinary_and_its_cut_off_container_thaws() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let key = c.alloc_node(ENVSXP, nil, nil, nil).unwrap();
            let w = c.alloc_vector(WEAKREFSXP, 4).unwrap();
            w.set_elt(0, key);
            w.set_elt(1, nil);
            w.set_elt(2, nil);
            c.register_weak_ref(w);
            let mid = c.alloc_node(LISTSXP, key, nil, nil).unwrap();
            let container = c.alloc_node(LISTSXP, mid, nil, nil).unwrap();
            let other = c.alloc_vector(INTSXP, 1).unwrap();
            let rest = c.alloc_node(LISTSXP, other, nil, nil).unwrap();
            let root = c.alloc_node(LISTSXP, container, rest, nil).unwrap();
            let mut roots = WithFreeze { all: vec![root], freeze: root };
            c.freeze(&mut roots, |x| x == mid);
            assert!(!c.is_frozen(key), "the weak key is never frozen");
            assert!(c.is_frozen(container) && (*c.remembered.0.get()).contains(&container), "the container is logged");
            assert!(c.is_frozen(other));
            c.check_frozen();
            cut_car(root, nil);
            c.frozen_pass(&mut roots);
            assert!(!c.is_frozen(container) && !(*c.remembered.0.get()).contains(&container));
            assert!(c.is_frozen(other), "the rest stays frozen");
            c.check_frozen();
        }
    }

    /// An external-data vector that a frozen container reaches through an
    /// excluded ordinary object, with no weak reference anywhere (Sol r4
    /// S3): the vector stays ordinary and the container is logged; cut off,
    /// the container thaws at the next pass, so the vector's release waits
    /// only for its reachability.
    #[test]
    fn external_data_stays_ordinary_and_its_cut_off_container_thaws() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        let mut buf = [0.0f64; 2];
        // SAFETY: fresh objects of this heap; `buf` outlives the heap's use
        // of it (no release callback; nothing collects).
        unsafe {
            let ext = c.alloc_extdata(REALSXP, buf.as_mut_ptr() as *mut u8, 2, 0, None).unwrap();
            let mid = c.alloc_node(LISTSXP, ext, nil, nil).unwrap();
            let container = c.alloc_node(LISTSXP, mid, nil, nil).unwrap();
            let other = c.alloc_vector(INTSXP, 1).unwrap();
            let rest = c.alloc_node(LISTSXP, other, nil, nil).unwrap();
            let root = c.alloc_node(LISTSXP, container, rest, nil).unwrap();
            let mut roots = WithFreeze { all: vec![root], freeze: root };
            c.freeze(&mut roots, |x| x == mid);
            assert!(!c.is_frozen(ext), "external data is never frozen");
            assert!(c.is_frozen(container) && (*c.remembered.0.get()).contains(&container));
            cut_car(root, nil);
            c.frozen_pass(&mut roots);
            assert!(!c.is_frozen(container) && !(*c.remembered.0.get()).contains(&container));
            assert!(c.is_frozen(other));
            c.check_frozen();
        }
    }

    /// A weak reference's key at the freeze is never frozen, so its death
    /// stays observable; the frozen objects on the path to it are logged,
    /// and its reachability through them is the key's, as in GNU R.
    #[test]
    fn a_weak_key_stays_ordinary_and_its_referrers_are_logged() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let key = c.alloc_node(ENVSXP, nil, nil, nil).unwrap();
            let w = c.alloc_vector(WEAKREFSXP, 4).unwrap();
            w.set_elt(0, key);
            w.set_elt(1, nil);
            w.set_elt(2, nil);
            c.register_weak_ref(w);
            let mid = c.alloc_node(LISTSXP, key, nil, nil).unwrap();
            let top = c.alloc_node(LISTSXP, mid, nil, nil).unwrap();
            let other = c.alloc_vector(INTSXP, 1).unwrap();
            let rest = c.alloc_node(LISTSXP, other, nil, nil).unwrap();
            let root = c.alloc_node(LISTSXP, top, rest, nil).unwrap();
            c.freeze(&mut One(root), |_| false);
            assert!(!c.is_frozen(key) && key.flags() & flags::IMMORTAL == 0, "the key is ordinary");
            assert!(c.is_frozen(mid) && c.is_frozen(top) && c.is_frozen(other));
            assert!((*c.remembered.0.get()).contains(&mid), "the key's frozen referrer is logged");
            c.check_frozen();
        }
    }

    /// A store into a frozen object over an ordinary value, and a store into
    /// an ordinary object over a frozen value, note nothing: neither removes
    /// an edge of the frozen graph. A frozen value overwritten in a frozen
    /// object does (the pass's bar, table (b)).
    #[test]
    fn only_a_frozen_value_overwritten_in_a_frozen_object_is_dirty() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let cell = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, cell, nil).unwrap();
            c.freeze(&mut One(head), |_| false);
            assert_eq!(c.remembered.1.get(), 0);
            let young = c.alloc_vector(INTSXP, 1).unwrap();
            cell.store(cell.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, young);
            assert_eq!(c.remembered.1.get() & Remembered::DIRTY, 0, "over nil (not frozen): no note");
            let other = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            other.store(other.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, nil);
            assert_eq!(c.remembered.1.get() & Remembered::DIRTY, 0, "in an ordinary object: no note");
            head.store(head.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, young);
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "a frozen value overwritten in a frozen object");
        }
    }

    /// I2 catches an edge cut without a note (a write that bypasses the
    /// barrier, as a missing row of the bar's table (b) would): the cut-off
    /// frozen object is reported unreachable.
    #[test]
    #[should_panic(expected = "is unreachable from the freeze roots")]
    fn the_check_reports_a_frozen_edge_cut_without_a_note() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            panic!("is unreachable from the freeze roots (a moving mode: the freeze is off)");
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            c.check_frozen_reach(&mut roots);
            // the barrier bypassed: no note
            *(head.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp) = nil;
            c.check_frozen_reach(&mut roots);
        }
    }

    /// A RAWSXP marked `NO_FREEZE` (C1's translation-cache stub) stays
    /// ordinary, and its frozen holder is logged; replacing it is no dirty
    /// event (an ordinary value overwritten). Another RAWSXP is frozen.
    #[test]
    fn a_no_freeze_raw_vector_stays_ordinary_and_its_replacement_is_not_dirty() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let stub = c.alloc_vector(RAWSXP, 16).unwrap();
            stub.set_extra(stub.extra() | crate::extra::vector::NO_FREEZE);
            let plain = c.alloc_vector(RAWSXP, 16).unwrap();
            let holder = c.alloc_node(LISTSXP, stub, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, plain, holder, nil).unwrap();
            c.freeze(&mut One(head), |_| false);
            assert!(!c.is_frozen(stub) && stub.flags() & flags::IMMORTAL == 0, "the marked stub stays ordinary");
            assert!(c.is_frozen(plain), "an unmarked RAWSXP is frozen");
            assert!(c.is_frozen(holder) && (*c.remembered.0.get()).contains(&holder), "its holder is logged");
            let code = c.alloc_vector(VECSXP, 1).unwrap();
            holder.store(holder.as_ptr().add(crate::layout::node::OFF_CAR) as *mut Sexp, code);
            assert_eq!(c.remembered.1.get() & Remembered::DIRTY, 0, "the stub's replacement is not dirty");
            c.check_frozen();
        }
    }

    /// A frozen object made immortal for good (a symbol's PRINTNAME: Sol r1
    /// S1) is pinned: no pass thaws it, even cut off from the freeze roots.
    #[test]
    fn a_frozen_object_made_immortal_is_pinned() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            c.make_immortal(leaf);
            assert!(!c.is_frozen(leaf) && leaf.flags() & flags::IMMORTAL != 0, "pinned");
            cut_car(head, nil);
            c.frozen_pass(&mut roots);
            assert!(leaf.flags() & flags::IMMORTAL != 0, "a pinned object is never thawed");
            c.check_frozen_reach(&mut roots);
        }
    }

    /// Shortening a frozen vector notes dirty (Sol r1 S4); a pass given no
    /// freeze roots thaws nothing (Sol r1 S5).
    #[test]
    fn a_shortened_frozen_vector_is_dirty_and_a_pass_without_freeze_roots_thaws_nothing() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let v = c.alloc_vector(VECSXP, 2).unwrap();
            v.set_elt(0, nil);
            v.set_elt(1, leaf);
            c.freeze(&mut One(v), |_| false);
            assert!(c.is_frozen(leaf));
            v.set_std_len(1);
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "a frozen vector shortened");
            c.frozen_pass(&mut One(v));
            assert!(c.is_frozen(v) && c.is_frozen(leaf), "no freeze roots: nothing thawed");
        }
    }

    /// The mover's OLD, the bit the freeze calls FROZEN, is cleared when the
    /// heap stops moving (Sol r1 S3).
    #[test]
    #[cfg(not(miri))]
    fn stopping_the_mover_clears_its_old_bit() {
        let mut c = Core::new();
        let nil = c.nil();
        // SAFETY: fresh objects of this heap, rooted by `roots`.
        unsafe {
            c.set_gc_move(true);
            let head = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            let mut roots = One(head);
            c.collect(&mut roots);
            let mut moved = 0usize;
            c.for_each_live_object(|p| {
                if *p.add(header::OFF_FLAGS) & flags::FROZEN != 0 {
                    moved += 1;
                }
            });
            assert!(moved > 0, "the moving collection marked its copies OLD");
            c.set_gc_move(false);
            let mut old = 0usize;
            c.for_each_live_object(|p| {
                if *p.add(header::OFF_FLAGS) & flags::FROZEN != 0 {
                    old += 1;
                }
            });
            assert_eq!(old, 0, "no object keeps the mover's OLD");
        }
    }

    /// A collection whose roots show no freeze roots consumes nothing: the
    /// dirty note survives it, and the next collection with freeze roots
    /// runs the pass (Sol r2 S5).
    #[test]
    fn a_pass_without_freeze_roots_leaves_the_dirty_note() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            cut_car(head, nil);
            let passes = c.frozen_passes();
            c.frozen_pass(&mut One(head));
            assert_eq!(c.frozen_passes(), passes, "no freeze roots: no pass");
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "and the note survives");
            c.frozen_pass(&mut roots);
            assert_eq!(c.frozen_passes(), passes + 1);
            assert!(!c.is_frozen(leaf), "the cut-off object thawed at the next pass that could decide");
            c.check_frozen_reach(&mut roots);
        }
    }

    /// An object made immortal while it holds a frozen one is logged, so
    /// that once a pass thaws that child, ordinary marking still traces it
    /// through the holder (Sol r2 S8).
    #[test]
    fn a_new_immortal_holder_of_a_frozen_object_is_logged() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            let holder = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            c.make_immortal(holder);
            assert!((*c.remembered.0.get()).contains(&holder), "logged: it holds a frozen object");
            cut_car(head, nil);
            c.frozen_pass(&mut roots);
            assert!(!c.is_frozen(leaf), "the child thawed");
            assert!((*c.remembered.0.get()).contains(&holder), "and its immortal holder stays logged: a root");
        }
    }

    /// Pinning a frozen container (made immortal for good) takes it out of
    /// the frozen graph: a dirty note, and it is logged, so that a pass that
    /// thaws its child leaves the child traced through it (Sol r2 S8).
    #[test]
    fn pinning_a_frozen_container_is_dirty_and_logs_it() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let container = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, container, nil, nil).unwrap();
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            assert_eq!(c.remembered.1.get(), 0);
            c.make_immortal(container);
            assert!(!c.is_frozen(container), "pinned");
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "pinning is a dirty event");
            assert!((*c.remembered.0.get()).contains(&container), "and the pinned container is logged");
            c.frozen_pass(&mut roots);
            assert!(!c.is_frozen(leaf), "its child, no longer frozen-reachable, thawed");
            assert!(container.flags() & flags::IMMORTAL != 0, "the pinned container stays immortal");
            c.check_frozen_reach(&mut roots);
        }
    }

    /// A root set with a search path too (`trace_search_path`).
    struct WithPath {
        all: Vec<Sexp>,
        freeze: Sexp,
        path: Vec<Sexp>,
    }
    impl Trace for WithPath {
        fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
            for x in &mut self.all {
                v.edge(x);
            }
        }
        fn trace_freeze<V: Visitor + ?Sized>(&mut self, v: &mut V) -> bool {
            v.edge(&mut self.freeze);
            for x in &mut self.path {
                v.edge(x);
            }
            true
        }
        fn trace_search_path<V: Visitor + ?Sized>(&mut self, v: &mut V) -> bool {
            for x in &mut self.path {
                v.edge(x);
            }
            true
        }
    }

    /// With no snapshot known (the freeze's roots showed no search path), a
    /// path write alone is a loss: the next pass whose roots show the search
    /// path runs (Sol r4 S13: the comparison, not the cut's own note).
    #[test]
    fn an_unknown_snapshot_makes_a_path_write_alone_run_the_pass() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let env = c.alloc_node(ENVSXP, nil, nil, nil).unwrap();
            let head = c.alloc_node(LISTSXP, nil, nil, nil).unwrap();
            c.freeze(&mut WithFreeze { all: vec![head, env], freeze: head }, |_| false);
            assert!(c.path_snapshot.is_none());
            let mut roots = WithPath { all: vec![head, env], freeze: head, path: vec![] };
            c.remembered.note(Remembered::PATH_WRITTEN);
            assert_eq!(c.remembered.1.get() & Remembered::DIRTY, 0, "a path write only");
            let passes = c.frozen_passes();
            c.frozen_pass(&mut roots);
            assert_eq!(c.frozen_passes(), passes + 1, "an unknown snapshot: the write counts as a loss");
            assert!(c.path_snapshot.is_some(), "and the snapshot is known from then on");
            // known now: a path write that loses nothing runs no pass
            c.remembered.note(Remembered::PATH_WRITTEN);
            c.frozen_pass(&mut roots);
            assert_eq!(c.frozen_passes(), passes + 1);
        }
    }

    /// A path write with roots that show no search path defers the pass,
    /// and the check does not count the deferred cut as missing (Sol r3
    /// S10); with no snapshot known, the next path write is a loss, so the
    /// pass runs (Sol r3 S11).
    #[test]
    fn a_deferred_pass_is_not_a_check_failure_and_an_unknown_snapshot_counts_a_path_write() {
        let mut c = Core::new();
        c.set_freeze_enabled(true);
        if !c.freeze_enabled() {
            return;
        }
        let nil = c.nil();
        // SAFETY: fresh objects of this heap; nothing collects.
        unsafe {
            let leaf = c.alloc_vector(INTSXP, 1).unwrap();
            let head = c.alloc_node(LISTSXP, leaf, nil, nil).unwrap();
            // freeze roots, no search path: the snapshot is unknown
            let mut roots = WithFreeze { all: vec![head], freeze: head };
            c.freeze(&mut roots, |_| false);
            assert!(c.path_snapshot.is_none(), "no search path: no snapshot");
            c.remembered.note(Remembered::PATH_WRITTEN);
            cut_car(head, nil);
            c.frozen_pass(&mut roots);
            assert!(c.is_frozen(leaf), "deferred: the roots cannot compare the search path");
            c.check_frozen_reach(&mut roots);
            assert_ne!(c.remembered.1.get() & Remembered::DIRTY, 0, "the notes wait");
        }
    }
}
