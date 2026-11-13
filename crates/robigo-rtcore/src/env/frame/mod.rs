//! SlotRef frames (docs/g3.md §5.1; the council's Fable round 2 §3.1-3.2):
//! every unhashed environment (closure frames, `new.env(hash = FALSE)`,
//! `R_NewEnv(.., FALSE, ..)`) is one ENVSXP with typed slots inline, one
//! per symbol of its shape, plus an overflow block for names outside the
//! shape (`assign`, `eval(quote(v <- 1), env)`, `list2env`,
//! `makeActiveBinding`, `<<-` creating, and every binding of a frame
//! without a shape). Shape transitions are D2s's measured variant, not
//! here. The word map is `layout` (`robigo_heap::slots`).
//!
//! The rules (§5.1, §6.3):
//!
//! - A slot is unbound (reserved by the shape, never defined, or removed
//!   by `rm`: lookup continues in the enclosure; listing does not see it),
//!   a value (an object, or an unboxed double, integer or logical), delayed
//!   or forced (an argument: expression, environment or value, PRSEEN and
//!   MISSING), active, or cell. Locked is a flag on any of them.
//! - **Listing order is GNU R's** (`defineVar` prepends, `envir.c:1998`):
//!   a slot gets a definition stamp when it becomes bound, never on
//!   reassignment, and bindings list by stamp descending. Formals are
//!   stamped in reverse formal order at call entry, so they list last and
//!   in formal order; `rm` then reinsertion moves a binding to the front.
//! - Reading a slot as an object boxes an unboxed scalar and stores the box
//!   back (`R_expand_binding_value`), and turns a delayed or forced slot
//!   into a value holding the PROMSXP GNU R's binding would hold, once
//!   (`mod.rs`: `ensure_object`).
//! - `R_findVarLocInFrame` makes a binding cell-backed for good: the slot
//!   points to a GNU R binding cell, and every read and write goes there.
//!
//! Functions here are raw: they take live objects of one isolate's heap
//! and neither allocate nor collect, except where a function says it takes
//! a freshly made object from its caller.

#![allow(unsafe_code)]

pub(crate) mod layout;

use super::chain;
use layout::*;
use robigo_heap::Sexp;

/// The slot keyed by `sym` in frame `f` (inline, then overflow), bound or
/// not.
///
/// # Safety
/// `f` a live slot frame, `sym` a live symbol.
#[inline]
pub(crate) unsafe fn find_slot(f: Sexp, sym: Sexp, nil: Sexp) -> Option<SlotRef> {
    let want = sym.addr() as u64;
    // one key compare per slot, walking the keys at the slot stride
    let scan = |o: Sexp, n: usize| {
        // SAFETY: `o` is a live slot frame or overflow block with `n` slots
        // (the caller's contract), so every key read is inside it.
        unsafe {
            let mut k = o.0.add(OFF_SLOTS + SLOT_KEY) as *const u64;
            for index in 0..n {
                if *k & !KIND_MASK == want {
                    return Some(SlotRef { owner: o, index });
                }
                k = k.add(SLOT_SIZE / 8);
            }
        }
        None
    };
    // SAFETY: the caller's contract; an overflow block's slots in use are
    // keyed or unkeyed, all initialized.
    unsafe {
        if let Some(s) = scan(f, capacity(f)) {
            return Some(s);
        }
        let b = get(f, OFF_OVERFLOW);
        if b != nil { scan(b, counter(b) as usize) } else { None }
    }
}

/// The slot binding `sym` in frame `f`, if bound (any kind but unbound).
///
/// # Safety
/// As for [`find_slot`].
#[inline]
pub(crate) unsafe fn find_bound(f: Sexp, sym: Sexp, nil: Sexp) -> Option<SlotRef> {
    // SAFETY: the caller's contract.
    unsafe { find_slot(f, sym, nil).filter(|s| s.kind() != UNBOUND) }
}

/// The next definition stamp of frame `f`, advancing it. Stamps only
/// order bindings, so when the counter would wrap the bound slots are
/// renumbered from 1 in their order.
///
/// # Safety
/// `f` is a live slot frame.
pub(crate) unsafe fn next_stamp(f: Sexp, nil: Sexp) -> u32 {
    // SAFETY: the caller's contract.
    unsafe {
        let n = counter(f);
        if n == u32::MAX {
            let mut b = bindings(f, nil);
            b.reverse(); // oldest first
            for (i, s) in b.iter().enumerate() {
                s.set_meta((s.meta() & !META_STAMP) | (i as u64 + 1));
            }
            let n = b.len() as u32 + 1;
            set_counter(f, n + 1);
            return n;
        }
        set_counter(f, n + 1);
        n
    }
}

/// Bind the unbound slot `s` (of frame `f`) to the object `v` as a value:
/// GNU R's `SET_FRAME(rho, CONS(value, FRAME(rho)))`, the newest binding.
///
/// # Safety
/// `f` the live slot frame `s` belongs to (inline or through its overflow
/// block), `s` unbound and keyed by `sym`, `v` live.
pub(crate) unsafe fn bind(f: Sexp, s: SlotRef, sym: Sexp, v: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract: an unbound slot holds no object, so
    // the key can change to VALUE before its `val` is stored.
    unsafe {
        let stamp = next_stamp(f, nil);
        s.set_meta(stamp as u64);
        s.set_key(sym, VALUE);
        s.store_val(v);
    }
}

/// A free overflow slot of `f` for `sym`, if its block has room (`None`:
/// the caller makes a block of [`grown_capacity`] slots and passes it to
/// [`adopt_overflow`]).
///
/// # Safety
/// `f` a live slot frame that does not bind `sym`, `sym` a live symbol.
pub(crate) unsafe fn overflow_slot(f: Sexp, sym: Sexp, nil: Sexp) -> Option<SlotRef> {
    // SAFETY: the caller's contract.
    unsafe {
        let b = get(f, OFF_OVERFLOW);
        if b == nil {
            return None;
        }
        let used = counter(b) as usize;
        if used == capacity(b) {
            return None;
        }
        set_counter(b, used as u32 + 1);
        let s = SlotRef { owner: b, index: used };
        s.set_key(sym, UNBOUND);
        Some(s)
    }
}

/// The capacity of the block that replaces `f`'s full (or missing)
/// overflow block: room for its bound slots and as many again, 4 at least.
///
/// # Safety
/// `f` is a live slot frame.
pub(crate) unsafe fn grown_capacity(f: Sexp, nil: Sexp) -> usize {
    // SAFETY: the caller's contract.
    unsafe {
        let b = get(f, OFF_OVERFLOW);
        if b == nil {
            return 4;
        }
        let live = (0..counter(b) as usize).filter(|&i| SlotRef { owner: b, index: i }.kind() != UNBOUND).count();
        ((live + 1) * 2).max(4)
    }
}

/// Make the fresh block `new` (from `alloc_slot_frame(cap, nil)`, not yet
/// published) `f`'s overflow block: the bound slots of the old one move
/// into it in order, their references with them (no count changes; the
/// old block, now garbage, is left with no slot bound).
///
/// # Safety
/// `f` a live slot frame, `new` a fresh empty block with room for the old
/// block's bound slots.
pub(crate) unsafe fn adopt_overflow(f: Sexp, new: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract; the slot words are copied raw (a
    // move: the old block's copies are unkeyed right after, so each
    // reference is counted once), and `new` is fresh, so no barrier applies
    // to its words.
    unsafe {
        let old = get(f, OFF_OVERFLOW);
        let mut used = 0;
        if old != nil {
            for i in 0..counter(old) as usize {
                let s = SlotRef { owner: old, index: i };
                if s.kind() == UNBOUND {
                    continue;
                }
                let d = SlotRef { owner: new, index: used };
                let src = old.0.add(OFF_SLOTS + i * SLOT_SIZE) as *const u64;
                let dst = new.0.add(OFF_SLOTS + used * SLOT_SIZE) as *mut u64;
                std::ptr::copy_nonoverlapping(src, dst, SLOT_SIZE / 8);
                debug_assert_eq!(d.sym(), s.sym());
                // unkey the old slot without dropping what moved
                s.set_key(Sexp::NULL, UNBOUND);
                used += 1;
            }
            set_counter(old, 0);
        }
        set_counter(new, used as u32);
        set(f, OFF_OVERFLOW, new);
    }
}

/// Store the object `v` as the value of the bound slot `s`, whose kind is
/// a value (boxed or not), delayed or forced: the store of GNU R's
/// `SET_BINDING_VALUE` after its lock and active checks. The slot becomes
/// a value; GNU R's `FIX_BINDING_REFCNT` keeps the old value's count once
/// while `ASSIGNMENT_PENDING` is set. Whether the caller environment is
/// still needed is the caller's to settle ([`release_caller`]).
///
/// # Safety
/// `s` a bound slot of a live frame, of one of those kinds; `v` live.
// R: memory.c:SET_BNDCELL (FIX_BINDING_REFCNT)
pub(crate) unsafe fn store_value(s: SlotRef, v: Sexp) {
    // SAFETY: the caller's contract; each word is cleared or stored by what
    // its current kind holds.
    unsafe {
        let sym = s.sym();
        match s.kind() {
            REAL | INT | LGL => {
                s.clear_raw_val();
                s.set_key(sym, VALUE);
                s.store_val(v);
            }
            VALUE => {
                let old = s.val();
                if old != v && s.has(META_PENDING) {
                    s.set_flag(META_PENDING, false);
                    old.incref();
                }
                s.store_val(v);
            }
            DELAYED | FORCED => {
                s.store_aux(Sexp::NULL);
                s.set_flag(META_ORIGIN_SELF, false);
                s.set_meta(s.meta() & !(3 << META_PRSEEN_SHIFT));
                s.set_key(sym, VALUE);
                s.store_val(v);
            }
            k => unreachable!("robigo: store_value on a slot of kind {k}"),
        }
    }
}

/// Write an unboxed scalar (`kind` REAL, INT or LGL, `bits` its value)
/// into the bound or unbound slot `s` whose kind is a value or unbound:
/// the VM's store of a local (V1). Any object the slot held loses its
/// count.
///
/// # Safety
/// `s` a slot of a live frame, of kind VALUE, REAL, INT, LGL or UNBOUND
/// (an unbound one already stamped by the caller); `kind` one of the three.
pub(crate) unsafe fn store_scalar(s: SlotRef, kind: u64, bits: u64) {
    // SAFETY: the caller's contract.
    unsafe {
        let sym = s.sym();
        if s.kind() == VALUE {
            s.store_val(Sexp::NULL);
        }
        s.set_key(sym, kind);
        s.set_val_bits(bits);
    }
}

/// Remove the binding in slot `s`: GNU R's `RemoveFromList` on a frame. A
/// cell-backed binding's cell is detached as GNU R detaches a removed cell
/// (it holds R_UnboundValue, locked, "in case binding is cached"). The slot
/// becomes unbound and keeps its key.
///
/// # Safety
/// `s` a bound slot of a live frame; `unbound` is R_UnboundValue.
// R: envir.c:RemoveFromList
pub(crate) unsafe fn unbind(s: SlotRef, unbound: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        let sym = s.sym();
        match s.kind() {
            REAL | INT | LGL => s.clear_raw_val(),
            CELL => {
                let c = s.val();
                chain::set_value(c, unbound);
                chain::lock_cell(c);
                s.store_val(Sexp::NULL);
            }
            _ => {
                s.store_val(Sexp::NULL);
                s.store_aux(Sexp::NULL);
            }
        }
        s.set_meta(0);
        s.set_key(sym, UNBOUND);
    }
}

/// Clear `f`'s caller word once no delayed slot evaluates there any more:
/// GNU R drops a promise's environment when it is forced (`SET_PRENV(e,
/// R_NilValue)`), so the caller's frame is not kept by the callee's.
///
/// # Safety
/// `f` is a live slot frame.
pub(crate) unsafe fn release_caller(f: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if get(f, OFF_CALLER) == nil {
            return;
        }
        let needed = (0..capacity(f)).any(|index| {
            let s = SlotRef { owner: f, index };
            s.kind() == DELAYED && !s.has(META_ORIGIN_SELF)
        });
        if !needed {
            set(f, OFF_CALLER, nil);
        }
    }
}

/// Every bound slot of `f` in GNU R's listing order: definition stamp
/// descending (`FrameNames`, `FrameValues` walk the prepended pairlist).
///
/// # Safety
/// `f` is a live slot frame.
pub(crate) unsafe fn bindings(f: Sexp, nil: Sexp) -> Vec<SlotRef> {
    // SAFETY: the caller's contract.
    unsafe {
        let mut out: Vec<SlotRef> = (0..capacity(f)).map(|index| SlotRef { owner: f, index }).collect();
        let b = get(f, OFF_OVERFLOW);
        if b != nil {
            out.extend((0..counter(b) as usize).map(|index| SlotRef { owner: b, index }));
        }
        out.retain(|s| s.kind() != UNBOUND);
        out.sort_unstable_by_key(|s| std::cmp::Reverse(s.stamp()));
        out
    }
}
