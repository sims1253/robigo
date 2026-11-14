//! D5's variant (branch g3/x2-d5-recycle; m1-plan X2, docs/g3.md §5.3,
//! §16): **LIFO frame recycling under an ESCAPED bit.** A closure's slot
//! frame that nothing can reach when its call returns goes to a per-isolate
//! pool, one LIFO list per slot capacity, and the next closure frame of that
//! capacity is that frame, rewritten, instead of a new allocation.
//!
//! A frame can be reached after its call returns only through
//!
//! - a counted heap edge: its REFCNT, after `R_CleanupEnvir` has cleared
//!   the bindings (a closure, promise, environment or argument slot that
//!   holds it counts it);
//! - an uncounted reference: R code that had the frame as a value
//!   (`environment()`, `sys.frame()`, `parent.frame()`, `as.environment()`
//!   and every other read of a context's `cloenv` or `sysparent`, which mark
//!   it [`ESCAPED`]; the values then sit in registers, `CONS_NR` lists or
//!   `*tmp*`, where no count shows them), C (`SEEN_BY_C`), or the call's own
//!   value (the frame itself).
//!
//! So a frame is recycled only with REFCNT 0, neither bit set, not the
//! value returned, no attributes, no overflow block, no version box, and no
//! cell-backed or active slot (each of those is held elsewhere or was made
//! by code that saw the frame).
//!
//! The pool is weak: every collection empties it (its frames are garbage to
//! the collector, which may free or move them).
//!
//! **Quarantine mode** (`ROBIGO_FRAME_QUARANTINE=1`, docs/g3.md §16): a frame
//! that would be recycled is poisoned instead (no longer a slot frame nor a
//! hashed one, so any environment operation on it panics in `kind`) and
//! held back (a root, so its memory is never reused): a frame that escaped
//! without the bit is caught where it is used next.

#![allow(unsafe_code)]

use super::frame::layout::{self, CELL, OFF_CALLER, OFF_OVERFLOW, OFF_SHAPE, OFF_VERSION, SlotRef};
use robigo_heap::{Sexp, Trace, Visitor, flags};
use std::cell::UnsafeCell;

/// The header `extra` bit of a slot frame some R code or C had as a value
/// (bits 4 and 5 are robigo's other bits, 8-12 a promise's tag).
pub(crate) const ESCAPED: u16 = 1 << 7;

/// Mark `x` [`ESCAPED`] if it is a slot frame (any object may be passed).
pub(crate) fn mark_escaped(x: Sexp) {
    // SAFETY: a context record's environment words are live objects or
    // R_NilValue (records are precise roots); the bit is a header field.
    unsafe {
        if !x.is_null() && x.typ() == robigo_heap::types::ENVSXP && layout::is_slots(x) {
            x.set_extra(x.extra() | ESCAPED);
        }
    }
}

/// Frames of up to this many inline slots are pooled.
const MAX_CAPACITY: usize = 32;
/// At most this many frames wait per capacity.
const PER_CAPACITY: usize = 64;

/// The pool (and, in quarantine mode, the frames held back).
pub(crate) struct FramePool {
    lists: UnsafeCell<Vec<Vec<Sexp>>>,
    quarantine: UnsafeCell<Vec<Sexp>>,
    quarantine_mode: bool,
}

impl Default for FramePool {
    fn default() -> FramePool {
        FramePool {
            lists: UnsafeCell::new((0..=MAX_CAPACITY).map(|_| Vec::new()).collect()),
            quarantine: UnsafeCell::new(Vec::new()),
            quarantine_mode: std::env::var("ROBIGO_FRAME_QUARANTINE").is_ok_and(|v| !v.is_empty() && v != "0"),
        }
    }
}

/// The pool's frames are not roots: a collection drops them all. The
/// quarantined frames are.
impl Trace for FramePool {
    fn trace<V: Visitor + ?Sized>(&mut self, v: &mut V) {
        for l in self.lists.get_mut() {
            l.clear();
        }
        for x in self.quarantine.get_mut() {
            v.edge(x);
        }
    }
}

impl FramePool {
    pub(crate) fn for_each_root(&self, f: &mut dyn FnMut(Sexp)) {
        // SAFETY: a shared read; `f` gets copies and nothing it calls
        // reaches the pool (a collection runs with no `&EnvState` in use).
        let q: Vec<Sexp> = unsafe { (*self.quarantine.get()).clone() };
        for x in q {
            f(x);
        }
    }

    /// A pooled frame of `capacity` slots, if any (LIFO).
    pub(crate) fn take(&self, capacity: usize) -> Option<Sexp> {
        if capacity > MAX_CAPACITY {
            return None;
        }
        // SAFETY: a borrow that ends with this call, which calls nothing of
        // ours.
        unsafe { (&mut *self.lists.get())[capacity].pop() }
    }

    /// Offer the frame `f` of a call that returned `v` (null: no value).
    ///
    /// # Safety
    /// `f` is a live environment and `v` null or a live object, of the heap
    /// this pool's isolate owns; the call's `R_CleanupEnvir` and argument
    /// list cleanup have run.
    pub(crate) unsafe fn offer(&self, f: Sexp, v: Sexp, nil: Sexp) {
        // SAFETY: the caller's contract.
        unsafe {
            if f == v || !layout::is_slots(f) || f.refcnt() != 0 {
                return;
            }
            if f.extra() & (ESCAPED | robigo_heap::extra::SEEN_BY_C) != 0 || f.attrib_raw() != nil {
                return;
            }
            let cap = layout::capacity(f);
            if cap > MAX_CAPACITY {
                return;
            }
            for off in [OFF_OVERFLOW, OFF_VERSION, OFF_CALLER] {
                if layout::get(f, off) != nil {
                    return;
                }
            }
            if (0..cap).any(|index| matches!(SlotRef { owner: f, index }.kind(), CELL | layout::ACTIVE)) {
                return;
            }
            if self.quarantine_mode {
                // poisoned: neither a slot frame nor a hashed one (its
                // HASHTAB word, the shape's, is R_NilValue), so `kind`
                // panics on it; held back as a root
                layout::set(f, OFF_SHAPE, nil);
                f.set_flag(flags::SLOTS, false);
                (*self.quarantine.get()).push(f);
                return;
            }
            let l = &mut (&mut *self.lists.get())[cap];
            if l.len() < PER_CAPACITY {
                l.push(f);
            }
        }
    }
}

/// Make the pooled frame `f` look as `Core::alloc_slot_frame(capacity,
/// enclos)` makes a new one: every slot unkeyed and empty, the header bits
/// and counters cleared, ENCLOS `enclos`. The words that held objects hold
/// R_NilValue or null since its cleanup (cleared bindings, no overflow,
/// version box or caller), so they are overwritten without counts; its shape
/// is replaced by the caller.
///
/// # Safety
/// `f` came from [`FramePool::take`] in this interval (no collection
/// since), and `enclos` is a live environment of the same heap.
pub(crate) unsafe fn reinit(f: Sexp, enclos: Sexp) {
    // SAFETY: the caller's contract; slot words are written raw (they hold
    // R_NilValue, null or unboxed bits, none counted).
    unsafe {
        f.set_gp(0);
        f.set_extra(0);
        robigo_heap::env::set_enclos(f, enclos);
        for index in 0..layout::capacity(f) {
            let s = SlotRef { owner: f, index };
            s.set_key(Sexp::NULL, layout::UNBOUND);
            s.clear_raw_val();
            s.clear_raw_aux();
            s.set_meta(0);
        }
        layout::set_counter(f, 0);
    }
}
