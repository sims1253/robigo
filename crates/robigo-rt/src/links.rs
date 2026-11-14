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
    // the marks nest: a commit's release comes before its enclosing one's
    debug_assert!(m.top.get() >= old, "robigo: stack links released out of order");
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

/// Links raised on values held in value-stack slots while later work runs
/// (`evalList`'s and `evalListKeepMissing`'s values, `R_DispatchOrEvalSP`'s
/// object, `applydefine`'s right-hand side), dropped on every path out:
/// by [`Linked::release`] where GNU R drops them, or when the guard is
/// dropped on an early exit (GNU R's jumps reset the stack's protection as
/// they pass it, `R_BCProtReset`). The objects are read from their slots
/// when the links drop, so a moving collection between is harmless.
pub struct Linked {
    held: Held,
}

enum Held {
    /// The first `n` slots from `base` (each linked by [`Linked::link_next`]).
    Slots {
        base: std::ptr::NonNull<robigo_rtcore::Slot>,
        n: usize,
    },
    /// The cars of the pairlist in slot `head` (`CONS_NR` cells), each
    /// linked as it was appended.
    List {
        head: std::ptr::NonNull<robigo_rtcore::Slot>,
    },
    Done,
}

impl Linked {
    /// A guard for slots from `base`, none linked yet.
    ///
    /// # Safety
    /// The slots stay live value-stack slots until the guard is released or
    /// dropped (a scope's, closed after the guard goes), and each slot
    /// [`Linked::link_next`] counts holds an object then.
    pub unsafe fn slots(base: std::ptr::NonNull<robigo_rtcore::Slot>) -> Linked {
        Linked { held: Held::Slots { base, n: 0 } }
    }

    /// A guard for the values of the `CONS_NR` list rooted in `head`, whose
    /// cars the caller links as it appends them.
    ///
    /// # Safety
    /// As for [`Linked::slots`]: the slot stays live, and holds the list (or
    /// nothing) until the guard goes.
    pub unsafe fn list(head: robigo_rtcore::Root<'_>) -> Linked {
        let p = std::ptr::NonNull::new(head.as_ptr() as *mut robigo_rtcore::Slot).unwrap();
        Linked { held: Held::List { head: p } }
    }

    /// `INCREMENT_LINKS` of the object in the next slot.
    ///
    /// # Safety
    /// That slot holds a live object.
    pub unsafe fn link_next(&mut self) {
        if let Held::Slots { base, n } = &mut self.held {
            // SAFETY: the caller's contract.
            unsafe {
                if let Some(x) = base.add(*n).read().as_object() {
                    increment_links(x);
                }
            }
            *n += 1;
        }
    }

    /// Drop the links now (where GNU R's code drops them).
    pub fn release(mut self) {
        self.drop_links();
    }

    fn drop_links(&mut self) {
        match std::mem::replace(&mut self.held, Held::Done) {
            Held::Slots { base, n } => {
                for i in 0..n {
                    // SAFETY: the constructor's contract: live slots, each
                    // linked, read now (after any collection).
                    unsafe {
                        if let Some(x) = base.add(i).read().as_object() {
                            decrement_links(x);
                        }
                    }
                }
            }
            Held::List { head } => {
                // SAFETY: the constructor's contract: the slot holds the
                // list's head (or nothing), its cells live and their cars
                // linked; R_NilValue ends it.
                unsafe {
                    let Some(mut c) = head.read().as_object() else { return };
                    while c.typ() == robigo_heap::types::LISTSXP {
                        decrement_links(c.car());
                        c = c.cdr();
                    }
                }
            }
            Held::Done => {}
        }
    }
}

impl Drop for Linked {
    fn drop(&mut self) {
        self.drop_links();
    }
}
