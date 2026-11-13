//! The raw layer of the evaluator core (docs/g3.md §2.1: "raw |
//! `robigo_heap::Sexp` | heap, rtapi, the evaluator core | audited
//! `unsafe`"): what `robigo-rt`'s VM needs beyond the safe facade, each
//! function with its contract. The VM keeps every object it holds across a
//! safepoint in a value-stack slot ([`crate::Scope::frame`]), so these
//! functions hand out raw `Sexp`s that the caller roots before its next
//! `&mut Cx` call.

#![allow(unsafe_code)]

use crate::isolate::Cx;
use robigo_heap::{NeedGc, Sexp};
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
}
