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
//! (`DECREMENT_BCSTACK_LINKS`). The marks are value-stack slot indices.
//!
//! # What the stack is
//!
//! GNU R's node stack holds what its interpreters keep pending between
//! operations: byte code's operands and call frames, and nothing of C's
//! (a builtin's locals and `PROTECT`s are never linked). Here that is the
//! registers of the live VM activations, which each [`ActLink`] lists
//! (pushed and popped by `vm::exec`), from [`R_FIRST`](crate::ir::R_FIRST)
//! on (the environment, the code object and the scratch registers are
//! `bcEval`'s locals). Two kinds of slots are not on it:
//!
//! - **Rust roots** (a builtin's, the evaluator's own scopes): GNU R's C
//!   locals. `vapply` holding `X` while `FUN` runs, then naming the result
//!   by it, sees an in-place write to `X`'s binding as GNU R's does.
//! - **An executing builtin's argument registers** ([`ActLink::skip`]):
//!   GNU R links a builtin's arguments while it evaluates the later ones
//!   (`evalList`'s `INCREMENT_LINKS`, byte code's `PUSHCALLARG`) and drops
//!   the links before it calls the builtin (`evalList`'s `DECREMENT_LINKS`,
//!   `BUILTIN_CALL_FRAME_ARGS`, eval.c:5970), so while the builtin runs R
//!   code its arguments count nothing.
//!
//! Where GNU R counts a value held across R code explicitly (`do_for`'s
//! sequence, `applydefine`'s right-hand side, `endcontext`'s return value,
//! `R_DispatchOrEvalSP`'s object, `evalList`'s values), robigo counts it
//! explicitly too ([`Linked`], or at the site); docs/ledger/g3/
//! 2026-11-16-c1-link-walk.md lists them.

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_rtcore::Cx;
use std::cell::Cell;

/// `R_BCProtTop` and `R_BCProtCommitted`, as value-stack depths, and the
/// live activations (the innermost first).
#[derive(Clone, Debug)]
pub struct Marks {
    top: Cell<usize>,
    committed: Cell<usize>,
    acts: Cell<*const ActLink>,
}

impl Default for Marks {
    fn default() -> Marks {
        Marks { top: Cell::new(0), committed: Cell::new(0), acts: Cell::new(std::ptr::null()) }
    }
}

/// A live VM activation's registers on the node stack: the value-stack
/// slots `lo..hi`, less the argument registers of a builtin it is running
/// (`skip`). On the activation's own Rust frame, listed from
/// [`Marks::acts`] while it runs ([`enter`], [`ActGuard`]).
#[derive(Debug)]
pub struct ActLink {
    /// The stack index of register 0.
    r0: usize,
    /// The node stack's part: registers `R_FIRST..nregs`, as stack indices.
    lo: usize,
    hi: usize,
    skip: Cell<(usize, usize)>,
    prev: *const ActLink,
}

impl ActLink {
    /// The link of an activation whose `nregs` registers are the newest
    /// value-stack slots.
    pub fn new(cx: &Cx, nregs: usize) -> ActLink {
        let hi = cx.stack_depth();
        let r0 = hi - nregs;
        let lo = r0 + (crate::ir::R_FIRST as usize).min(nregs);
        ActLink { r0, lo, hi, skip: Cell::new((0, 0)), prev: std::ptr::null() }
    }

    /// The activation's registers `first..first + n`, the arguments of a
    /// builtin it calls, are off the node stack until the guard goes
    /// (`BUILTIN_CALL_FRAME_ARGS`'s `DECREMENT_LINKS`, eval.c:5970).
    pub fn skip(&self, first: crate::ir::Reg, n: usize) -> SkipGuard<'_> {
        let a = self.r0 + first as usize;
        let old = self.skip.replace((a, a + n));
        SkipGuard { link: self, old }
    }
}

/// Restores an activation's skipped registers ([`ActLink::skip`]).
pub struct SkipGuard<'a> {
    link: &'a ActLink,
    old: (usize, usize),
}

impl Drop for SkipGuard<'_> {
    fn drop(&mut self) {
        self.link.skip.set(self.old);
    }
}

/// Lists `link` as the innermost live activation until the guard goes
/// (on every path out of the activation, a panic's included).
///
/// # Safety
/// `link` stays where it is until the guard is dropped; the guards of one
/// isolate are dropped in the reverse order of their making; the
/// isolate's evaluator state is not replaced meanwhile (`define_builtins`
/// asserts no activation is listed).
pub unsafe fn enter<'a>(cx: &Cx, link: &'a mut ActLink) -> ActGuard<'a> {
    let m = &crate::prims::state(cx).links;
    link.prev = m.acts.get();
    m.acts.set(link as *const ActLink);
    ActGuard { marks: m as *const Marks, link }
}

/// Unlists an activation ([`enter`]).
pub struct ActGuard<'a> {
    marks: *const Marks,
    link: &'a ActLink,
}

impl ActGuard<'_> {
    /// The listed link.
    pub fn link(&self) -> &ActLink {
        self.link
    }
}

impl Drop for ActGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `enter`'s contract: the evaluator state the marks are in
        // outlives every listed activation.
        let m = unsafe { &*self.marks };
        debug_assert!(std::ptr::eq(m.acts.get(), self.link), "robigo: activations unlisted out of order");
        m.acts.set(self.link.prev);
    }
}

impl Marks {
    /// Whether an activation is listed (the evaluator state is replaced
    /// only when none is).
    pub fn any_live(&self) -> bool {
        !self.acts.get().is_null()
    }
}

/// `INCLNK_stack`/`DECLNK_stack` over the node stack's slots `from..to`:
/// each live activation's registers in that range (less those a running
/// builtin's arguments occupy) gain or lose a link per object.
///
/// # Safety
/// `from..to` is below the stack's depth, and the activations listed hold
/// what they held when the range was linked (for a decrement).
// R: eval.c:INCLNK_stack_commit, eval.c:DECLNK_stack
unsafe fn walk(cx: &Cx, from: usize, to: usize, inc: bool) {
    let mut p = crate::prims::state(cx).links.acts.get();
    // SAFETY: every listed link is on a live activation's frame (its guard
    // unlists it before the frame goes); the caller's contract for the
    // slots.
    unsafe {
        while let Some(a) = p.as_ref() {
            // the activations nest on the stack: older ones lie lower
            if a.hi <= from {
                break;
            }
            let (lo, hi) = (a.lo.max(from), a.hi.min(to));
            if lo < hi {
                let (s0, s1) = a.skip.get();
                if s1 <= s0 || s1 <= lo || s0 >= hi {
                    cx.stack_links(lo, hi, inc);
                } else {
                    if lo < s0 {
                        cx.stack_links(lo, s0, inc);
                    }
                    if s1 < hi {
                        cx.stack_links(s1, hi, inc);
                    }
                }
            }
            p = a.prev;
        }
    }
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
        unsafe { walk(cx, c, t, true) };
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
            // changed (the activations and frames below it are suspended,
            // each in the same instruction, its skipped registers too).
            unsafe { walk(cx, old, c, false) };
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
