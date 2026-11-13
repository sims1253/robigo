//! Chains of binding cells: GNU R's pairlists whose nodes are bindings
//! (CAR the value, TAG the symbol, gp the binding bits). A hashed
//! environment's table holds one chain per slot; a cell-backed slot of a
//! slot frame points to one such cell (`frame/`).
//!
//! Every function here is raw: it takes live objects and stores through
//! `Sexp::store` (the barrier and GNU R's reference counts), and it neither
//! allocates nor collects.

#![allow(unsafe_code)]

use crate::sym::{ACTIVE_BINDING, ASSIGNMENT_PENDING, BINDING_LOCK};
use robigo_heap::{Sexp, flags};

/// GNU R's `MISSING_MASK` on a binding cell's gp.
pub(crate) const MISSING_MASK: u16 = 15;

/// Store `v` as a binding cell's value: GNU R's `SETCAR` on a binding,
/// whose `FIX_BINDING_REFCNT` keeps the old value's count while the
/// cell's `ASSIGNMENT_PENDING` is set (a complex assignment in progress,
/// C1), clearing the bit instead.
///
/// # Safety
/// `cell` is a live binding cell, `v` a live object.
// R: memory.c:SETCAR (FIX_BINDING_REFCNT)
pub(crate) unsafe fn set_value(cell: Sexp, v: Sexp) {
    // SAFETY: the caller's contract; the store keeps the barrier and the
    // counts, and the extra increment cancels its decrement of `old`.
    unsafe {
        let old = cell.car();
        let g = cell.gp();
        if old != v && g & ASSIGNMENT_PENDING != 0 && cell.tracks_refs() {
            cell.set_gp(g & !ASSIGNMENT_PENDING);
            old.incref();
        }
        cell.set_car(v);
    }
}

/// The cell binding `sym` in `chain`, if any.
///
/// # Safety
/// `chain` is a live chain of binding cells ending in `nil`.
#[inline]
pub(crate) unsafe fn find(mut chain: Sexp, sym: Sexp, nil: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract: every node is a live LISTSXP.
    unsafe {
        while chain != nil {
            if chain.tag() == sym {
                return Some(chain);
            }
            chain = chain.cdr();
        }
    }
    None
}

/// GNU R's `MARK_NOT_MUTABLE`: the count saturated, so the value is
/// duplicated before any change.
///
/// # Safety
/// `x` is live.
#[inline]
pub(crate) unsafe fn mark_not_mutable(x: Sexp) {
    // SAFETY: the caller's contract. Immortal objects keep their count.
    unsafe {
        if !x.has_flag(flags::IMMORTAL) {
            x.set_refcnt(u16::MAX);
        }
    }
}

/// GNU R's `LOCK_BINDING` on a cell: the value is marked not mutable
/// unless the binding is active, then the lock bit is set.
///
/// # Safety
/// `cell` is a live binding cell.
// R: Defn.h:LOCK_BINDING
pub(crate) unsafe fn lock_cell(cell: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        let gp = cell.gp();
        if gp & ACTIVE_BINDING == 0 {
            mark_not_mutable(cell.car());
        }
        cell.set_gp(gp | BINDING_LOCK);
    }
}

/// GNU R's `SET_MISSING(cell, 0)`.
///
/// # Safety
/// `cell` is a live binding cell.
#[inline]
pub(crate) unsafe fn clear_missing(cell: Sexp) {
    // SAFETY: the caller's contract.
    unsafe { cell.set_gp(cell.gp() & !MISSING_MASK) }
}

/// GNU R's `RemoveFromList`: unlink the cell binding `thing` from `list`
/// and return the list's new head, and whether it was found. The removed
/// cell is left holding `R_UnboundValue`, locked and detached, "in case
/// binding is cached" (a global cache entry or a varloc still names it).
///
/// # Safety
/// `list` is a live chain of binding cells ending in `nil`; `unbound` is
/// `R_UnboundValue`.
// R: envir.c:RemoveFromList
pub(crate) unsafe fn remove(thing: Sexp, list: Sexp, nil: Sexp, unbound: Sexp) -> (Sexp, bool) {
    // SAFETY: the caller's contract: every node is a live LISTSXP, and every
    // store keeps GNU R's counts (`Sexp::store`).
    unsafe {
        if list == nil {
            return (nil, false);
        }
        if list.tag() == thing {
            set_value(list, unbound); // SET_BNDCELL
            lock_cell(list);
            let rest = list.cdr();
            list.set_cdr(nil); // to fix refcnt on 'rest'
            return (rest, true);
        }
        let mut last = list;
        let mut next = list.cdr();
        while next != nil {
            if next.tag() == thing {
                set_value(next, unbound);
                lock_cell(next);
                last.set_cdr(next.cdr());
                next.set_cdr(nil); // to fix refcnt on 'list'
                return (list, true);
            }
            last = next;
            next = next.cdr();
        }
        (list, false)
    }
}
