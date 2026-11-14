//! Link counts for values held on the stack (docs/g3.md §4: "Deferred link
//! counts for stack values, ported from `eval.c:997-1016`"). C1 owns this
//! module; V1b's call sites (complex assignment, `evalList`, `[`'s
//! dispatch) go through it.
//!
//! GNU R's values on the byte-code node stack count no references; before
//! an operation that may mutate an object in place (`applydefine`,
//! `STARTASSIGN`), the stack's values get one link each
//! (`INCREMENT_BCSTACK_LINKS` and `INCLNK_stack_commit`), so that an object
//! a pending computation holds looks shared and is copied rather than
//! written. The links are dropped when the operation ends
//! (`DECREMENT_BCSTACK_LINKS`). Here the stack is the value stack: every
//! activation's registers and every root. The marks are slot indices.

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_rtcore::Cx;
use std::cell::Cell;

/// `R_BCProtTop` and `R_BCProtCommitted`, as value-stack depths.
#[derive(Default, Clone, Debug)]
pub struct Marks {
    top: Cell<usize>,
    committed: Cell<usize>,
}

/// `INCREMENT_LINKS(x)` (`INCREMENT_NAMED` is a no-op under reference
/// counting).
///
/// # Safety
/// `x` is a live object.
#[inline]
pub unsafe fn increment_links(x: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { x.incref() }
}

/// `DECREMENT_LINKS(x)`.
///
/// # Safety
/// `x` is a live object whose links an earlier [`increment_links`] raised.
#[inline]
pub unsafe fn decrement_links(x: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { x.decref() }
}

/// `INCREMENT_BCSTACK_LINKS(); INCLNK_stack_commit();`: every value on the
/// stack now holds a link. Returns the old protection mark, for
/// [`release`].
// R: eval.c:INCREMENT_BCSTACK_LINKS, eval.c:INCLNK_stack_commit
pub fn commit(cx: &Cx) -> usize {
    let m = &crate::prims::state(cx).links;
    let old = m.top.get();
    let depth = cx.stack_depth();
    if depth > m.top.get() {
        m.top.set(depth);
    }
    let (c, t) = (m.committed.get(), m.top.get());
    if c < t {
        // SAFETY: `c..t` lies below the stack's depth (the marks never
        // exceed it while an operation that set them is running).
        unsafe { cx.stack_links(c, t, true) };
        m.committed.set(t);
    }
    old
}

/// `DECREMENT_BCSTACK_LINKS(old)`: the links [`commit`] gave above the old
/// mark are dropped.
// R: eval.c:DECREMENT_BCSTACK_LINKS, eval.c:DECLNK_stack
pub fn release(cx: &Cx, old: usize) {
    let m = &crate::prims::state(cx).links;
    if m.top.get() > old {
        let c = m.committed.get();
        if old < c {
            // SAFETY: the slots `old..c` are those `commit` linked: the
            // operation that holds them has not returned, so they have not
            // changed (the activations and frames below it are suspended).
            unsafe { cx.stack_links(old, c, false) };
            m.committed.set(old);
        }
        m.top.set(old);
    }
}
