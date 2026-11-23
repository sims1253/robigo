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
        // the stopped objects whose death is observable: a weak reference's
        // key, an external-data vector (its release callback)
        let mut mortal_targets = Vec::new();
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
                    || exclude(x)
                    || boot.is_some_and(|l| !self.is_boot_object(x, l));
                if never {
                    stopped.insert(x.addr());
                    if observable {
                        mortal_targets.push(x);
                    }
                    continue;
                }
                self.make_immortal(x);
                if x.typ() != CHARSXP {
                    x.set_refcnt(FROZEN_REFCNT);
                }
                frozen.push(x);
                kids.clear();
                x.trace_fields(&mut Edges(&mut kids));
                work.extend_from_slice(&kids);
            }
        }
        frozen.sort_unstable_by_key(|x| x.addr());
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
        // then what can reach an object whose death is observable stays
        // ordinary, so that its reachability, not a log entry, decides (Sol
        // S3): every weak reference's key, whether the walk met it or not,
        // and the external-data vectors (the thaw finds those behind an
        // ordinary object too: r3)
        for &w in &self.weak_refs {
            // SAFETY: a registered weak reference: element 0 is its key.
            mortal_targets.push(unsafe { Self::weak_field(w, 0) });
        }
        // SAFETY: as above; the log is complete.
        unsafe { self.thaw_with_referrers(&mut mortal_targets) };
        self.frozen.len()
    }

    /// Make `targets` (frozen or not) and every frozen object that reaches
    /// one of them ordinary: one pass over the frozen set builds its
    /// reverse edges, and a walk up from the targets finds the referrers.
    /// No frozen object then points to one of them, so no log entry is
    /// needed and ordinary reachability decides their death.
    ///
    /// # Safety
    /// Frozen objects are live and their fields null or live; outside
    /// marking.
    unsafe fn thaw_with_referrers(&mut self, targets: &mut Vec<Sexp>) {
        // (with no target the walk below still runs while anything is
        // logged: it finds the external-data vectors behind ordinary objects,
        // r4 S3)
        // SAFETY: outside marking, the log is not borrowed elsewhere.
        if targets.is_empty() && unsafe { &*self.remembered.0.get() }.is_empty() {
            return;
        }
        let mut parents: HashMap<usize, Vec<Sexp>, BuildFx> = HashMap::default();
        let mut kids = Vec::new();
        for &f in &self.frozen {
            kids.clear();
            // SAFETY: the caller's contract.
            unsafe { f.trace_fields(&mut Edges(&mut kids)) };
            for k in &kids {
                parents.entry(k.addr()).or_default().push(f);
            }
        }
        // and through the ordinary objects the logged ones reach: a frozen
        // container can reach a target through an ordinary one (Sol r2 S3)
        // SAFETY: outside marking, the log is not borrowed elsewhere.
        let mut work: Vec<Sexp> = unsafe { &*self.remembered.0.get() }.clone();
        let mut seen: HashSet<usize, BuildFx> = HashSet::default();
        while let Some(o) = work.pop() {
            kids.clear();
            // SAFETY: the log's objects and what they reach are live.
            unsafe { o.trace_fields(&mut Edges(&mut kids)) };
            for &k in &kids {
                // SAFETY: as above.
                let ordinary = unsafe { k.flags() } & flags::IMMORTAL == 0;
                if ordinary {
                    parents.entry(k.addr()).or_default().push(o);
                    if seen.insert(k.addr()) {
                        work.push(k);
                        // an external-data vector behind an ordinary object:
                        // its release callback is observable too (r3 S3)
                        // SAFETY: as above.
                        if unsafe { k.flags() } & flags::EXTDATA != 0 {
                            targets.push(k);
                        }
                    }
                } else if self.is_frozen(k) && !self.is_frozen(o) {
                    // an ordinary object's edge into the frozen set
                    parents.entry(k.addr()).or_default().push(o);
                }
            }
        }
        // walk up from the targets through frozen and ordinary referrers;
        // the frozen ones on the way thaw
        let mut thaw: HashSet<usize, BuildFx> = HashSet::default();
        let mut up: HashSet<usize, BuildFx> = HashSet::default();
        for t in targets.iter() {
            up.insert(t.addr());
        }
        while let Some(t) = targets.pop() {
            if self.is_frozen(t) {
                thaw.insert(t.addr());
            }
            if let Some(ps) = parents.get(&t.addr()) {
                for &p in ps {
                    if up.insert(p.addr()) {
                        targets.push(p);
                    }
                }
            }
        }
        for i in 0..self.frozen.len() {
            let f = self.frozen[i];
            if thaw.contains(&f.addr()) {
                // SAFETY: a live frozen object.
                unsafe { self.make_mortal(f) };
            }
        }
        self.frozen.retain(|f| !thaw.contains(&f.addr()));
        // a thawed object leaves the log (a logged object is a root, and an
        // ordinary one must be able to die), in one filter
        // SAFETY: outside marking, the log is not borrowed elsewhere.
        let log = unsafe { &mut *self.remembered.0.get() };
        log.retain(|r| !thaw.contains(&r.addr()));
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
                super::barrier_only(x);
            }
        }
    }

    /// Whether `x` is one of the freeze's objects.
    pub(crate) fn is_frozen(&self, x: Sexp) -> bool {
        self.is_frozen_at(x.addr())
    }

    /// Whether the object at `addr` is one of the freeze's objects (an
    /// address compared, nothing read: any address may be asked).
    pub fn is_frozen_at(&self, addr: usize) -> bool {
        self.frozen.binary_search_by_key(&addr, |f| f.addr()).is_ok()
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
        if self.is_frozen(x) {
            self.pending_unfreeze.push(x);
        }
    }

    /// The batched unfreeze: the queued objects and every frozen object
    /// that reaches one of them become ordinary. One pass over the frozen
    /// set, however long the queue.
    ///
    /// # Safety
    /// At a collection's start, before marking.
    pub(super) unsafe fn unfreeze_pending(&mut self) {
        if self.pending_unfreeze.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending_unfreeze);
        pending.sort_unstable_by_key(|x| x.addr());
        pending.dedup();
        // SAFETY: the caller's contract.
        unsafe { self.thaw_with_referrers(&mut pending) };
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
            x.set_flag_raw(flags::IMMORTAL | flags::BARRIER, false);
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

    /// A weak reference made after the freeze on a key that a frozen
    /// container reaches through an ordinary object (Sol r2 S3): the unfreeze
    /// thaws the key and that container, which leaves the log, so only
    /// reachability decides.
    #[test]
    fn the_unfreeze_thaws_a_container_reaching_the_key_through_an_ordinary_object() {
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
            c.freeze(&mut One(root), |x| x == mid);
            assert!(c.is_frozen(key) && c.is_frozen(container) && !c.is_frozen(mid));
            assert_eq!(container.flags() & flags::BARRIER, 0, "the container reaches the ordinary `mid`: logged");
            let w = c.alloc_vector(WEAKREFSXP, 4).unwrap();
            w.set_elt(0, key);
            w.set_elt(1, nil);
            w.set_elt(2, nil);
            c.register_weak_ref(w);
            c.unfreeze_pending();
            assert!(!c.is_frozen(key) && key.flags() & flags::IMMORTAL == 0, "the key thawed");
            assert!(!c.is_frozen(container) && container.flags() & flags::IMMORTAL == 0, "and its container");
            let log = &*c.remembered.0.get();
            assert!(!log.contains(&container), "which left the log");
            c.check_frozen();
        }
    }

    /// A weak reference registered before the freeze on a key that only a
    /// frozen container reaches, through an excluded ordinary object (Sol r3
    /// S3): the container comes out of the freeze ordinary, unlogged.
    #[test]
    fn the_freeze_thaws_a_container_reaching_an_earlier_weak_key_through_an_ordinary_object() {
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
            c.freeze(&mut One(root), |x| x == mid);
            assert!(!c.is_frozen(container) && container.flags() & flags::IMMORTAL == 0, "the container is ordinary");
            assert!(!c.is_frozen(root), "and what reaches it");
            assert!(c.is_frozen(other), "the rest is frozen");
            let log = &*c.remembered.0.get();
            assert!(!log.contains(&container), "the container is not logged");
            c.check_frozen();
        }
    }

    /// An external-data vector that only a frozen container reaches, through
    /// an excluded ordinary object, with no weak reference anywhere (Sol r4
    /// S3): the container comes out of the freeze ordinary, unlogged, so the
    /// vector's release waits only for its reachability.
    #[test]
    fn the_freeze_thaws_a_container_reaching_external_data_through_an_ordinary_object() {
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
            c.freeze(&mut One(root), |x| x == mid);
            assert!(!c.is_frozen(container) && container.flags() & flags::IMMORTAL == 0, "the container is ordinary");
            assert!(c.is_frozen(other));
            let log = &*c.remembered.0.get();
            assert!(!log.contains(&container));
            c.check_frozen();
        }
    }

    /// A weak reference's key at the freeze: neither it nor any frozen
    /// object that reaches it is frozen, so its death stays observable.
    #[test]
    fn a_weak_key_and_its_referrers_stay_ordinary() {
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
            for x in [key, mid, top, root] {
                assert!(!c.is_frozen(x) && x.flags() & flags::IMMORTAL == 0, "on the path to the key");
            }
            assert!(c.is_frozen(other), "off it");
            c.check_frozen();
        }
    }
}
