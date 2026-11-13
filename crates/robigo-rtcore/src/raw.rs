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

    /// A vector of type `t` and length `n` (plain data zeroed, pointer
    /// elements R_NilValue or `""`), under `&Cx`: never collects. The result
    /// is fresh and must be rooted before the next safepoint.
    #[inline]
    pub fn alloc_vector_raw(&self, t: u8, n: usize) -> Result<Sexp, NeedGc> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.alloc_vector(t, n)
    }

    /// Whether `x` (a live object) belongs to this isolate's heap: the
    /// owner check, for the raw layer's own assertions.
    #[inline(always)]
    pub fn owns_raw(&self, x: Sexp) -> bool {
        self.iso().owns(x)
    }

    /// A full collection (`R_gc()`, `gc()`'s, a `Explicit` safepoint), with
    /// the isolate's precise roots. Takes `&mut Cx`: no `Ref` survives it.
    pub fn collect(&mut self) {
        let iso = self.iso();
        // SAFETY: as in `Cx::safepoint`: `&mut Cx` has ended every `Ref`,
        // view and slice, so everything still needed is in a root of this
        // isolate; the collection reaches nothing of ours.
        unsafe { iso.core().collect(iso.roots_mut()) };
    }

    /// Whether weak references wait for their finalizers
    /// (`R_finalizers_pending`).
    pub fn finalizers_pending(&self) -> bool {
        // SAFETY: one read of the core.
        unsafe { self.iso().core() }.pending_finalizer_count() > 0
    }

    /// The next weak reference whose finalizer is due (GNU R's
    /// `RunFinalizers` order): no longer rooted by the queue once popped,
    /// so the caller roots it before its next safepoint.
    pub fn pop_finalizable_raw(&mut self) -> Option<Sexp> {
        // SAFETY: one call into the core, which calls nothing of ours.
        unsafe { self.iso().core() }.pop_finalizable()
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
                let g = w.gp() & !gp::READY_TO_FINALIZE;
                w.set_gp(if onexit { g | gp::FINALIZE_ON_EXIT } else { g & !gp::FINALIZE_ON_EXIT });
                iso.core().register_weak_ref(w);
            }
            Ok(w)
        }
    }

    /// GNU R's link counts for the values on the stack (eval.c:997-1056):
    /// `INCREMENT_LINKS` (`inc`) or `DECREMENT_LINKS` of the object in each
    /// value-stack slot `from..to` (indices from the base; unboxed and empty
    /// slots hold none). The VM's registers and the roots are its
    /// `R_BCNodeStack`; the evaluator keeps the committed range
    /// (`R_BCProtCommitted`).
    ///
    /// # Safety
    /// `from..to` lies below [`Cx::stack_depth`]; a decrement undoes an
    /// increment of the same objects (the slots have not changed since).
    pub unsafe fn stack_links(&self, from: usize, to: usize, inc: bool) {
        self.iso().stack().for_each_object_in(from, to, &mut |x| {
            // SAFETY: an object slot holds a live object of this heap; the
            // count is a header field.
            unsafe { if inc { x.incref() } else { x.decref() } }
        });
    }

    /// GNU R's `ASSIGNMENT_PENDING(cell)` of a binding cell from
    /// [`Cx::find_var_loc_in_frame`] (a symbol: a base binding).
    pub fn assignment_pending(&self, cell: crate::Ref<'_>) -> bool {
        assert!(matches!(cell.sexptype(), types::LISTSXP | types::SYMSXP), "robigo: not a binding cell");
        // SAFETY: a live object; gp is a header field.
        unsafe { cell.sexp().gp() & ASSIGNMENT_PENDING != 0 }
    }

    /// GNU R's `SET_ASSIGNMENT_PENDING(cell, on)` of a binding cell.
    pub fn set_assignment_pending(&mut self, cell: crate::Root<'_>, on: bool) {
        let c = cell.bind(self);
        assert!(matches!(c.sexptype(), types::LISTSXP | types::SYMSXP | types::NILSXP), "robigo: not a binding cell");
        if c.sexptype() == types::NILSXP {
            return;
        }
        let x = c.sexp();
        // SAFETY: a rooted cell; the bit changes no body.
        unsafe {
            let g = x.gp();
            x.set_gp(if on { g | ASSIGNMENT_PENDING } else { g & !ASSIGNMENT_PENDING });
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

/// GNU R's `ASSIGNMENT_PENDING_MASK` (Defn.h).
const ASSIGNMENT_PENDING: u16 = 1 << 11;
