//! The boot heap's freeze (docs/ledger/g3/2026-11-21-h-boot-freeze-bar.md).
//!
//! After the embedder's boot and one full collection, [`Core::freeze`]
//! makes every object reachable from the roots it is given immortal, so
//! that no later collection traces it again. An immortal object is never
//! marked (marking stops at it) and never swept (the sweep keeps it by its
//! flag, and a block whose objects are all marked or immortal skips its
//! bitmap walk).
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

use super::*;

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

impl Core {
    /// Whether the freeze runs in this heap: not under the moving
    /// collector's modes.
    pub fn freeze_enabled(&self) -> bool {
        !self.gc_move && mover::policy_from_env().is_none()
    }

    /// How many objects the freeze made immortal and are still frozen.
    pub fn frozen_count(&self) -> usize {
        self.frozen.len()
    }

    /// Freeze every object reachable from `roots` (see the module's doc),
    /// stopping at the objects `exclude` names and at the kinds that are
    /// never frozen. Returns how many objects it froze. Run once, after a
    /// full collection, so that only survivors are frozen. Nothing under
    /// the moving collector's modes.
    ///
    /// # Safety
    /// Every edge `roots` shows is null or a live object of this heap, as
    /// for [`Core::collect`]; nothing else runs on the heap meanwhile.
    pub unsafe fn freeze<R: Trace + ?Sized>(&mut self, roots: &mut R, exclude: impl Fn(Sexp) -> bool) -> usize {
        if !self.freeze_enabled() || !self.frozen.is_empty() {
            return 0;
        }
        let mut work = Vec::new();
        roots.trace(&mut Edges(&mut work));
        // the keys of weak references: their death is observable
        let mut keys: std::collections::HashSet<Sexp, BuildFx> = std::collections::HashSet::default();
        for &w in &self.weak_refs {
            // SAFETY: a registered weak reference: element 0 is its key.
            keys.insert(unsafe { Self::weak_field(w, 0) });
        }
        let mut stopped: std::collections::HashSet<Sexp, BuildFx> = std::collections::HashSet::default();
        let mut frozen = Vec::new();
        let mut kids = Vec::new();
        while let Some(x) = work.pop() {
            // SAFETY: the caller's contract: `x` is null or live.
            unsafe {
                if x.is_null() || x.flags() & flags::IMMORTAL != 0 || stopped.contains(&x) {
                    continue;
                }
                let never = x.typ() == WEAKREFSXP
                    || x.extra() & crate::extra::SEEN_BY_C != 0
                    || keys.contains(&x)
                    || exclude(x);
                if never {
                    stopped.insert(x);
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
        // a frozen object with an ordinary child is a root from now on
        for &f in &frozen {
            // SAFETY: frozen objects are live; their fields are null or live.
            unsafe {
                if self.has_ordinary_child(f, &mut kids) {
                    self.remember(f);
                }
            }
        }
        frozen.sort_unstable_by_key(|x| x.addr());
        let n = frozen.len();
        self.frozen = frozen;
        n
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
            if x.flags() & flags::BARRIER != 0 && !crate::shared::is_shared(x) {
                super::barrier_only(x);
            }
        }
    }

    /// Whether `x` is one of the freeze's objects.
    pub fn is_frozen(&self, x: Sexp) -> bool {
        self.frozen.binary_search_by_key(&x.addr(), |f| f.addr()).is_ok()
    }

    /// Queue a frozen object to become ordinary at the next collection (a
    /// weak reference keyed on it is being made). Nothing for any other
    /// object.
    pub fn request_unfreeze(&mut self, x: Sexp) {
        if self.is_frozen(x) && !self.pending_unfreeze.contains(&x) {
            self.pending_unfreeze.push(x);
        }
    }

    /// The batched unfreeze: log every frozen referrer of a queued object,
    /// then make the queued objects ordinary. One pass over the frozen set.
    ///
    /// # Safety
    /// At a collection's start, before marking.
    pub(super) unsafe fn unfreeze_pending(&mut self) {
        if self.pending_unfreeze.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending_unfreeze);
        pending.sort_unstable_by_key(|x| x.addr());
        let mut kids = Vec::new();
        // SAFETY: frozen objects are live, and so are their fields.
        unsafe {
            for i in 0..self.frozen.len() {
                let f = self.frozen[i];
                if pending.binary_search_by_key(&f.addr(), |p| p.addr()).is_ok() {
                    continue;
                }
                kids.clear();
                f.trace_fields(&mut Edges(&mut kids));
                if kids.iter().any(|k| pending.binary_search_by_key(&k.addr(), |p| p.addr()).is_ok()) {
                    self.remember(f);
                }
            }
            for &p in &pending {
                self.make_mortal(p);
            }
        }
        self.frozen.retain(|f| pending.binary_search_by_key(&f.addr(), |p| p.addr()).is_err());
    }

    /// Undo [`Core::make_immortal`] for a frozen object: an ordinary object
    /// again, unlogged. Its reference count stays at the frozen maximum
    /// (shared).
    ///
    /// # Safety
    /// `x` is a live frozen object, at a collection's start.
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
            // (an object logged before, when it was frozen, leaves the log:
            // a logged object is a root, and an ordinary one must be able to
            // die)
            let log = &mut *self.remembered.0.get();
            log.retain(|&r| r != x);
        }
    }

    /// `ROBIGO_GC_FREEZE_CHECK=1`: no frozen object that is not logged
    /// points to an ordinary object. That is the invariant, so everything
    /// an ordinary object reaches through a frozen one is reachable from a
    /// root. Panics, naming both objects, otherwise.
    ///
    /// # Safety
    /// Outside marking: frozen objects and their fields are live.
    pub unsafe fn check_frozen(&self) {
        let mut kids = Vec::new();
        for &f in &self.frozen {
            // SAFETY: the caller's contract.
            unsafe {
                if f.flags() & flags::BARRIER == 0 {
                    // logged: traced at every collection
                    continue;
                }
                kids.clear();
                f.trace_fields(&mut Edges(&mut kids));
                if let Some(k) = kids.iter().find(|k| k.flags() & flags::IMMORTAL == 0) {
                    panic!(
                        "robigo: ROBIGO_GC_FREEZE_CHECK: frozen {:#x} (type {}) points to the ordinary {:#x} (type {}) and is not logged",
                        f.addr(),
                        f.typ(),
                        k.addr(),
                        k.typ()
                    );
                }
            }
        }
    }
}
