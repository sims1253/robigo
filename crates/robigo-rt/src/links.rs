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
//! registers of the live VM activations from [`R_FIRST`](crate::ir::R_FIRST)
//! on (the environment, the code object and the scratch registers are
//! `bcEval`'s locals). `vm::exec` heads each activation's registers with
//! [`FRAME_HEAD`] marked slots ([`mark_frame`]), which nothing else on the
//! value stack holds, and the walk counts the registers that follow a head.
//! Two kinds of slots are not on it:
//!
//! - **Rust roots** (a builtin's, the evaluator's own scopes): GNU R's C
//!   locals. `vapply` holding `X` while `FUN` runs, then naming the result
//!   by it, sees an in-place write to `X`'s binding as GNU R's does.
//! - **An executing builtin's argument registers** ([`skip_args`], in the
//!   head's second slot): GNU R links a builtin's arguments while it
//!   evaluates the later ones (`evalList`'s `INCREMENT_LINKS`, byte code's
//!   `PUSHCALLARG`) and drops the links before it calls the builtin
//!   (`evalList`'s `DECREMENT_LINKS`, `BUILTIN_CALL_FRAME_ARGS`,
//!   eval.c:5970), so while the builtin runs R code its arguments count
//!   nothing.
//!
//! Where GNU R counts a value held across R code explicitly (`do_for`'s
//! sequence, `applydefine`'s right-hand side, `endcontext`'s return value,
//! `R_DispatchOrEvalSP`'s object, `evalList`'s values, a list's
//! `SET_VECTOR_ELT`), robigo counts it explicitly too ([`Linked`], or at
//! the site); docs/ledger/g3/2026-11-16-c1-link-walk.md lists them.

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_rtcore::{Cx, Slot};
use std::cell::Cell;
use std::ptr::NonNull;

/// `R_BCProtTop` and `R_BCProtCommitted`, as value-stack depths.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    top: Cell<usize>,
    committed: Cell<usize>,
}

/// The marked slots below an activation's registers: the head (its
/// register count), then the registers a builtin it runs holds its
/// arguments in (empty while it runs none).
pub const FRAME_HEAD: usize = 2;

/// The head's word: this bit and the register count. (The second slot's
/// word, the arguments' registers, never has it.)
const HEAD: u64 = 1 << 63;

/// Head a fresh frame of [`FRAME_HEAD`] + `nregs` slots as an activation's,
/// its registers after the head.
///
/// # Safety
/// `frame` is the first of those slots, empty, on the value stack.
#[inline(always)]
pub unsafe fn mark_frame(frame: NonNull<Slot>, nregs: usize) {
    // SAFETY: the caller's contract; a mark is no object.
    unsafe { frame.write(Slot::mark(HEAD | nregs as u64)) };
}

/// Registers `first..first + n` of the activation whose registers start
/// at `base`, the arguments of a builtin it calls, hold no links until
/// [`unskip_args`] (`BUILTIN_CALL_FRAME_ARGS`'s `DECREMENT_LINKS`,
/// eval.c:5970). No guard: an activation runs one builtin at a time and
/// stops while it does, and an unwind past it ends the activation, whose
/// frame goes.
///
/// # Safety
/// `base` is a running activation's register 0 ([`mark_frame`]'s frame).
#[inline(always)]
pub unsafe fn skip_args(base: NonNull<Slot>, first: crate::ir::Reg, n: usize) {
    // SAFETY: the caller's contract: the head's second slot.
    unsafe { base.sub(1).write(Slot::mark(first as u64 | (n as u64) << 32)) };
}

/// The builtin [`skip_args`] named has returned.
///
/// # Safety
/// As for [`skip_args`].
#[inline(always)]
pub unsafe fn unskip_args(base: NonNull<Slot>) {
    // SAFETY: the caller's contract.
    unsafe { base.sub(1).write(Slot::EMPTY) };
}

/// The frame headed at slot `h` (whose head's word is `w`): its registers
/// on the node stack, and those a running builtin's arguments occupy.
fn frame_at(cx: &Cx, h: usize, w: u64) -> ((usize, usize), (usize, usize)) {
    let nregs = (w & !HEAD) as usize;
    let r0 = h + FRAME_HEAD;
    let regs = (r0 + (crate::ir::R_FIRST as usize).min(nregs), r0 + nregs);
    let skip = match cx.stack_slot(h + 1).as_mark() {
        Some(a) => {
            let first = r0 + (a & 0xffff_ffff) as usize;
            (first, first + (a >> 32) as usize)
        }
        None => (0, 0),
    };
    (regs, skip)
}

/// `stack_links` of `lo..hi` less `skip`.
///
/// # Safety
/// As for [`walk`].
unsafe fn links_less(cx: &Cx, (lo, hi): (usize, usize), (s0, s1): (usize, usize), inc: bool) {
    // SAFETY: the caller's contract.
    unsafe {
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
}

/// `INCLNK_stack`/`DECLNK_stack` over the node stack's slots `from..to`:
/// the registers of each activation headed in that range, or whose head is
/// below it and registers reach into it (less those a running builtin's
/// arguments occupy), gain or lose a link per object. The frames do not
/// nest: an inner activation's lies above its caller's registers, past the
/// roots of what the caller runs.
///
/// # Safety
/// `from..to` is below the stack's depth, and its slots hold what they held
/// when the range was linked (for a decrement).
// R: eval.c:INCLNK_stack_commit, eval.c:DECLNK_stack
#[inline(never)]
unsafe fn walk(cx: &Cx, from: usize, to: usize, inc: bool) {
    // the frame `from` lies in, if any: the nearest head below it
    let mut i = from;
    let mut j = from;
    while j > 0 {
        j -= 1;
        if let Some(w) = cx.stack_slot(j).as_mark().filter(|w| w & HEAD != 0) {
            let (regs, skip) = frame_at(cx, j, w);
            if regs.1 > from {
                let r = (regs.0.max(from), regs.1.min(to));
                if r.0 < r.1 {
                    // SAFETY: the caller's contract.
                    unsafe { links_less(cx, r, skip, inc) };
                }
                i = regs.1;
            }
            break;
        }
    }
    // then each head in the range, its registers, and the roots between
    while i < to {
        if let Some(w) = cx.stack_slot(i).as_mark().filter(|w| w & HEAD != 0) {
            let (regs, skip) = frame_at(cx, i, w);
            let r = (regs.0, regs.1.min(to));
            if r.0 < r.1 {
                // SAFETY: the caller's contract.
                unsafe { links_less(cx, r, skip, inc) };
            }
            i = regs.1.max(i + 1);
        } else {
            i += 1;
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
