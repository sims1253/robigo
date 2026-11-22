//! The raw layer of the evaluator core (docs/g3.md §2.1: "raw |
//! `robigo_heap::Sexp` | heap, rtapi, the evaluator core | audited
//! `unsafe`"): what `robigo-rt`'s VM needs beyond the safe facade, each
//! function with its contract. The VM keeps every object it holds across a
//! safepoint in a value-stack slot ([`crate::Scope::frame`]), so these
//! functions hand out raw `Sexp`s that the caller roots before its next
//! `&mut Cx` call.

#![allow(unsafe_code)]

use crate::isolate::Cx;
use robigo_heap::{NeedGc, Sexp, gp, types};
use std::cell::Cell;

impl Cx {
    /// The evaluator's per-isolate state, if one of type `T` is installed.
    #[inline]
    pub fn ext<T: 'static>(&self) -> Option<&T> {
        self.iso().ext::<T>()
    }

    /// Install the evaluator's per-isolate state (replacing any). It may hold
    /// objects only through roots of this isolate (`Global`s) or immortal
    /// ones (symbols), and is dropped with the isolate.
    pub fn set_ext<T: 'static>(&mut self, v: T) {
        // SAFETY: `&mut Cx` ends every `&Cx` borrow, so no reference returned
        // by `ext` is live.
        unsafe { self.iso().set_ext(Box::new(v)) }
    }

    /// `eval`'s `evalcount` (eval.c:1085): one count per evaluation, the
    /// poll of GNU R's unit when it passes 1000 (docs/g3.md §3 item 7). A
    /// cell of the isolate, at a stable address: the VM ticks it in place.
    #[inline(always)]
    pub fn evalcount(&self) -> &Cell<i32> {
        &self.ctxs().evalcount
    }

    /// `bcEval`'s `evalcount` (eval.c:7538), polled past 1023
    /// (`BC_COUNT_DELTA`, eval.c:6250): the `Bc` profile's.
    #[inline(always)]
    pub fn bc_evalcount(&self) -> &Cell<i32> {
        &self.ctxs().bc_evalcount
    }

    /// A node (`LISTSXP`, `LANGSXP`, `PROMSXP`, `CLOSXP`, `DOTSXP`, ...)
    /// with the given fields, under `&Cx`: never collects. The 4th word is
    /// R_NilValue.
    ///
    /// # Safety
    /// `car`, `cdr` and `tag` are live objects of this isolate's heap,
    /// reachable from a root for this call (or immortal); `t` is a node
    /// type. The result is fresh and must be rooted before the next
    /// safepoint.
    #[inline]
    pub unsafe fn alloc_node_raw(&self, t: u8, car: Sexp, cdr: Sexp, tag: Sexp) -> Result<Sexp, NeedGc> {
        // SAFETY: the caller's contract; one call into the core, which calls
        // nothing of ours.
        unsafe { self.iso().core().alloc_node(t, car, cdr, tag) }
    }

    /// A vector of type `t` and length `n` (plain data not initialized,
    /// pointer elements R_NilValue or `""`), under `&Cx`: never collects.
    /// The result is fresh and must be rooted before the next safepoint.
    #[inline]
    pub fn alloc_vector_raw(&self, t: u8, n: usize) -> Result<Sexp, NeedGc> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.alloc_vector(t, n)
    }

    /// Whether `x` belongs to this isolate's heap: the owner check, for the
    /// raw layer's own assertions.
    ///
    /// # Safety
    /// `x` is a live object of some heap: its block header is read.
    #[inline(always)]
    pub unsafe fn owns_raw(&self, x: Sexp) -> bool {
        self.iso().owns(x)
    }

    /// A full collection (`R_gc()`, `gc()`'s, a `Explicit` safepoint), with
    /// the isolate's precise roots. Takes `&mut Cx`: no `Ref` survives it.
    pub fn collect(&mut self) {
        let iso = self.iso();
        // SAFETY: as in `Cx::safepoint`: `&mut Cx` has ended every `Ref`,
        // view and slice, so everything still needed is in a root of this
        // isolate; the collection reaches nothing of ours.
        unsafe { iso.core().collect(&mut iso.gc_roots()) };
        self.flush_gc_report();
    }

    /// Whether the boot heap's freeze runs here: `ROBIGO_GC_FREEZE=1`, and
    /// not under ROBIGO_GC_MOVE's modes.
    pub fn freeze_enabled(&self) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.freeze_enabled()
    }

    /// Turn the boot heap's freeze on or off before the boot boundary, as
    /// `ROBIGO_GC_FREEZE` does (the tests' switch).
    ///
    /// # Panics
    /// Once the boundary is drawn or anything is frozen.
    pub fn set_freeze_enabled(&mut self, on: bool) {
        // SAFETY: one call on the core, no collection.
        unsafe { self.iso().core() }.set_freeze_enabled(on)
    }

    /// How many objects are frozen.
    pub fn frozen_count(&self) -> usize {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.frozen_count()
    }

    /// Whether `x` is one of the freeze's objects.
    pub fn is_frozen(&self, x: crate::Ref<'_>) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.is_frozen_at(x.sexp().addr())
    }

    /// The freeze's invariant, checked (`ROBIGO_GC_FREEZE_CHECK`'s check):
    /// panics if a frozen object that is not logged points to an ordinary
    /// one.
    pub fn check_frozen(&self) {
        // SAFETY: outside a collection, frozen objects and their fields are
        // live; one read of the core.
        unsafe { self.iso().core().check_frozen() }
    }

    /// Log `x` as a root, once (robigo-heap's `Core::remember`): a frozen
    /// object whose fields C may write without the barrier.
    ///
    /// # Safety
    /// `x` is a live object of this isolate.
    pub unsafe fn remember_raw(&self, x: Sexp) {
        // SAFETY: the caller's contract; one call into the core, which
        // calls nothing of ours.
        unsafe { self.iso().core().remember(x) }
    }

    /// The boot heap's freeze (robigo-heap's `freeze.rs`,
    /// docs/ledger/g3/2026-11-21-h-boot-freeze-bar.md and its amendments),
    /// when `ROBIGO_GC_FREEZE=1` turns it on: the boot boundary, drawn by
    /// the front end once, at the end of the boot. Only the boot's blocks
    /// are marked here; the first collection after it freezes every boot
    /// object reachable from the freeze roots (the base environment and
    /// namespace, the symbols' cells, the namespace registry, the
    /// `Globals`, the C API's constants), the global environment and its
    /// frame left out. Nothing when the freeze is off (the default) or
    /// under the moving collector's modes.
    pub fn freeze_boot_heap(&mut self) {
        let iso = self.iso();
        // SAFETY: one read of the core.
        if !unsafe { iso.core() }.freeze_enabled() {
            return;
        }
        // a frame pooled before the boundary and taken after it would be a
        // session's frame at a boot object's address (Sol r9 S15): the pool
        // is emptied, as a collection empties it
        iso.envs().clear_frame_pool();
        let global = iso.envs().global_raw();
        // SAFETY: one call on the core, no collection: it marks blocks.
        unsafe { iso.core() }.freeze_boundary(global.addr());
    }

    /// Move every movable object at every collection from now on, or stop
    /// (`ROBIGO_GC_MOVE=1` sets it for every isolate: docs/g3.md §3
    /// "Checks"). A debug check: a raw `Sexp` kept across a safepoint
    /// outside the roots then reads the old copy's poison.
    ///
    /// # Safety
    /// As for [`robigo_heap::Core::set_gc_move`]: starting needs that C
    /// holds no address of an object of this isolate handed to it while
    /// moving was off.
    pub unsafe fn set_gc_move(&mut self, on: bool) {
        // SAFETY: one call into the core, which calls nothing of ours; the
        // condition is the caller's.
        unsafe { self.iso().core().set_gc_move(on) };
        // the C API's exports and accessors mark what they hand to C
        self.cglobals().gc_move.set(on as std::ffi::c_int);
    }

    /// GNU R's `R_finalizers_pending`: a collection (or the exit) left
    /// weak references ready, and no finalizer pass ran since.
    pub fn finalizers_pending(&self) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.finalizers_pending()
    }

    /// The end of a finalizer pass: what is still ready waits for the next
    /// collection (`RunFinalizers`' `R_finalizers_pending = FALSE`).
    pub fn end_finalizer_pass(&mut self) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.end_finalizer_pass()
    }

    /// `R_RunExitFinalizers`' marking: every weak reference registered
    /// with `onexit = TRUE` becomes due, in the weak-reference list's order.
    // R: memory.c:R_RunExitFinalizers
    pub fn queue_exit_finalizers(&mut self) {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.queue_exit_finalizers()
    }

    /// The next weak reference of a finalizer pass (GNU R's
    /// `RunFinalizers` walk): the newest ready one older than `before`, the
    /// position of the last one this pass took (`usize::MAX` to start), with
    /// its own position. No longer rooted by the list once taken, so the
    /// caller roots it before its next safepoint.
    pub fn pop_finalizable_raw(&mut self, before: usize) -> Option<(Sexp, usize)> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.pop_finalizable_before(before)
    }

    /// GNU R's `NewWeakRef(key, value, fin, onexit)` for a key of a
    /// reference type (the caller checks GNU R's "can only weakly
    /// reference/finalize reference objects", and duplicates a referenced
    /// value): a WEAKREFSXP registered with the collector.
    ///
    /// # Safety
    /// `key`, `value` and `fin` are live objects of this isolate, rooted by
    /// the caller; the result is rooted by the collector's list.
    // R: memory.c:NewWeakRef
    pub unsafe fn new_weak_ref_raw(&mut self, key: Sexp, value: Sexp, fin: Sexp, onexit: bool) -> Result<Sexp, NeedGc> {
        let iso = self.iso();
        // SAFETY: the caller's contract; the reference is fresh (its stores
        // take no barrier), and registering it makes the collector trace it.
        unsafe {
            let w = iso.core().alloc_vector(types::WEAKREFSXP, 4)?;
            if key != iso.nil {
                w.set_elt(0, key);
                w.set_elt(1, value);
                w.set_elt(2, fin);
                let g = w.gp() & !gp::weakref::READY_TO_FINALIZE;
                w.set_gp(if onexit { g | gp::weakref::FINALIZE_ON_EXIT } else { g & !gp::weakref::FINALIZE_ON_EXIT });
                iso.core().register_weak_ref(w);
            }
            Ok(w)
        }
    }

    /// `R_PreserveObject(x)`: `x` is a precise root until released (the
    /// heap's precious set).
    ///
    /// # Safety
    /// `x` is a live object of this isolate.
    pub unsafe fn preserve_raw(&mut self, x: Sexp) {
        assert!(self.iso().owns(x) || x == self.iso().nil, "robigo: an object of another isolate");
        // SAFETY: the caller's contract; one call into the core.
        unsafe { self.iso().core().preserve(x) }
    }

    /// `R_ReleaseObject(x)`: the most recent preservation of `x` ends (none:
    /// nothing happens, as in GNU R).
    ///
    /// # Safety
    /// As for [`robigo_heap::Core::release`].
    pub unsafe fn release_raw(&mut self, x: Sexp) {
        // SAFETY: one call into the core, which calls nothing of ours; `x`
        // is the caller's to vouch for.
        unsafe { self.iso().core().release(x) }
    }

    /// A vector of type `t` and length `n` where a collection may run first
    /// ([`Cx::alloc_rooted`], typed by its `SEXPTYPE`): for requests of
    /// [`robigo_heap::LARGE`] bytes or more. Plain data is zeroed, pointer
    /// elements are `""` or R_NilValue. The result is fresh: the caller
    /// roots it before its next safepoint.
    pub fn alloc_vector_rooted_raw(&mut self, t: u8, n: usize) -> crate::RResult<Sexp> {
        let r = {
            let iso = self.iso();
            // SAFETY: as in `Cx::alloc_rooted`: a collection may run here,
            // with the isolate's roots, while `&mut Cx` has ended every `Ref`.
            unsafe { iso.core().alloc_rooted(t, n, &mut iso.gc_roots()) }
        };
        self.flush_gc_report();
        r.map_err(|e| self.error_call(None, &e.message()))
    }

    /// GNU R's shared logical scalars `R_FalseValue`, `R_TrueValue` and
    /// `R_LogicalNAValue` (`ScalarLogical(x)` returns one, allocating
    /// nothing): never mutable (their count is saturated, so every write
    /// duplicates first).
    pub fn logical_value(&self, x: i32) -> crate::Ref<'_> {
        let i = if x == robigo_heap::NA_LOGICAL { 2 } else { (x != 0) as usize };
        // SAFETY: rooted by the C API's state, and never freed.
        unsafe { crate::Ref::from_sexp(self.iso().capi_logicals()[i], self) }
    }

    /// The value-stack slots `from..to` (indices from the base) as they are
    /// now, read in place: how robigo-rt's link walk finds the marks that
    /// head its activations' registers ([`crate::Slot::mark`]) and the
    /// objects in them, whose link counts it raises or drops (GNU R's
    /// `INCLNK_stack_commit` and `DECLNK_stack`, eval.c:997-1056). Panics
    /// unless `from <= to <=` [`Cx::stack_depth`].
    ///
    /// # Safety
    /// No slot of the range is written while the slice lives (a link count
    /// is the object's header field, not a slot).
    pub unsafe fn stack_slots(&self, from: usize, to: usize) -> &[crate::Slot] {
        // SAFETY: the caller's contract.
        unsafe { self.iso().stack().slots(from, to) }
    }

    /// GNU R's `ASSIGNMENT_PENDING(cell)` of a binding cell from
    /// [`Cx::find_var_loc_in_frame`] (a symbol: a base binding, whose bits
    /// are the isolate's symbol table's).
    pub fn assignment_pending(&self, cell: crate::Ref<'_>) -> bool {
        assert!(matches!(cell.sexptype(), types::LISTSXP | types::SYMSXP), "robigo: not a binding cell");
        let x = cell.sexp();
        // SAFETY: a live cell (gp is a header field) or a live symbol.
        unsafe {
            if cell.sexptype() == types::SYMSXP {
                self.iso().envs().base_assignment_pending(x)
            } else {
                x.gp() & ASSIGNMENT_PENDING != 0
            }
        }
    }

    /// GNU R's `SET_ASSIGNMENT_PENDING(cell, on)` of a binding cell.
    pub fn set_assignment_pending(&mut self, cell: crate::Root<'_>, on: bool) {
        let c = cell.bind(self);
        assert!(matches!(c.sexptype(), types::LISTSXP | types::SYMSXP | types::NILSXP), "robigo: not a binding cell");
        if c.sexptype() == types::NILSXP {
            return;
        }
        let x = c.sexp();
        // SAFETY: a rooted cell (the bit changes no body) or a live symbol,
        // which is never written (H5): its binding's bits are the table's.
        unsafe {
            if c.sexptype() == types::SYMSXP {
                self.iso().envs().set_base_assignment_pending(x, on);
            } else {
                let g = x.gp();
                x.set_gp(if on { g | ASSIGNMENT_PENDING } else { g & !ASSIGNMENT_PENDING });
            }
        }
    }

    /// `DISABLE_REFCNT(cell)` then `DECREMENT_REFCNT(CDR(cell))`: complex
    /// assignment's `*tmp*` binding counts neither its value nor the cell
    /// after it (eval.c:applydefine).
    pub fn disable_binding_refcnt(&mut self, cell: crate::Root<'_>) {
        let c = cell.bind(self);
        assert!(c.sexptype() == types::LISTSXP, "robigo: not a binding cell");
        let x = c.sexp();
        // SAFETY: a rooted cell: its flags and its cdr (a live object).
        unsafe {
            x.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
            x.cdr().decref();
        }
    }
}

use crate::sym::ASSIGNMENT_PENDING;
