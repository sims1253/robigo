//! GNU R's hashed frames, ported as written (`envir.c:200-560`): a VECSXP
//! of chains of binding cells (`chain.rs`), its TRUELENGTH the fill count
//! `HASHPRI`, chains indexed by `R_Newhashpjw(name) % HASHSIZE`. Listing
//! order, `env.profile()` and the bytes `serialize` writes follow from it.
//!
//! Two of GNU R's quirks are kept on purpose, since `env.profile()`'s
//! `nchains` and the resize points are observable: `R_HashSet` counts every
//! new binding into `HASHPRI`, not only one that starts a chain (its test
//! reads the loop variable after the loop, always `R_NilValue`), and
//! `R_HashDelete` uncounts only a chain that becomes empty. A resize
//! recounts.
//!
//! Raw functions: callers pass live objects; nothing here allocates, so
//! the new table and new cells are made by the caller (`mod.rs`, which
//! retries an allocation after a collection).

#![allow(unsafe_code)]

use super::chain;
use crate::sym::hash_of;
use robigo_heap::Sexp;

/// `HASHMINSIZE`: the size of a table asked for with size 0 (or less).
pub(crate) const HASHMINSIZE: usize = 29;
/// `HASHTABLEGROWTHRATE`.
const GROWTH_RATE: f64 = 1.2;

/// `HASHSIZE`.
///
/// # Safety
/// `table` is a live VECSXP.
#[inline(always)]
pub(crate) unsafe fn size(table: Sexp) -> usize {
    // SAFETY: the caller's contract.
    unsafe { table.std_len() as usize }
}

/// `HASHPRI`.
///
/// # Safety
/// `table` is a live VECSXP.
#[inline(always)]
pub(crate) unsafe fn pri(table: Sexp) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { table.truelen() as i32 }
}

/// # Safety
/// `table` is a live VECSXP.
#[inline(always)]
unsafe fn set_pri(table: Sexp, v: i32) {
    // SAFETY: the caller's contract.
    unsafe { table.set_truelen(v as isize) }
}

/// `HASHCHAIN(table, i)`.
///
/// # Safety
/// `table` is a live VECSXP and `i < size(table)`.
#[inline(always)]
pub(crate) unsafe fn chain_at(table: Sexp, i: usize) -> Sexp {
    // SAFETY: the caller's contract: element `i` of a pointer payload.
    unsafe { *(table.std_data() as *const Sexp).add(i) }
}

/// The chain `sym` hashes to (`hashIndex`).
///
/// # Safety
/// `sym` a live symbol, `table` a live, non-empty VECSXP.
#[inline(always)]
pub(crate) unsafe fn index(sym: Sexp, table: Sexp) -> usize {
    // SAFETY: the caller's contract.
    unsafe { hash_of(sym) as usize % size(table) }
}

/// `R_HashGetLoc`: the cell binding `sym`, if any.
///
/// # Safety
/// As for [`index`], and `table` is a hash table of chains.
// R: envir.c:R_HashGetLoc
#[inline]
pub(crate) unsafe fn get_loc(sym: Sexp, table: Sexp, nil: Sexp) -> Option<Sexp> {
    // SAFETY: the caller's contract.
    unsafe { chain::find(chain_at(table, index(sym, table)), sym, nil) }
}

/// The size `R_NewHashTable(size)` makes.
// R: envir.c:R_NewHashTable
pub(crate) fn new_size(size: i32) -> usize {
    if size <= 0 { HASHMINSIZE } else { size as usize }
}

/// The size `R_HashResize` grows `old_size` to.
// R: envir.c:R_HashResize
pub(crate) fn grown_size(old_size: usize) -> usize {
    1 + (old_size as f64 * GROWTH_RATE) as i32 as usize
}

/// The insertion half of `R_HashSet`: put `cell` (a new binding of a
/// symbol the table does not bind, made as `CONS(value, chain)` with its
/// TAG set) at the head of its chain, counting it into `HASHPRI`.
///
/// # Safety
/// `table` a live hash table; `cell` a live binding cell whose CDR is the
/// current head of chain `i`, where `i` is its symbol's [`index`].
// R: envir.c:R_HashSet
pub(crate) unsafe fn insert(table: Sexp, i: usize, cell: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        // GNU R tests `ISNULL(chain)` after its search loop, where `chain`
        // is always R_NilValue: every new binding counts
        set_pri(table, pri(table) + 1);
        table.set_elt(i, cell);
    }
}

/// `R_HashSizeCheck`: whether the table is due to grow.
///
/// # Safety
/// `table` is a live VECSXP.
// R: envir.c:R_HashSizeCheck
pub(crate) unsafe fn needs_resize(table: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { pri(table) as f64 > size(table) as f64 * 0.85 }
}

/// The moving half of `R_HashResize`: relink every cell of `old` into
/// `new` (a fresh, empty table), chain by chain, each at the head of its
/// new chain, counting the chains it starts.
///
/// # Safety
/// `old` a live hash table, `new` a live VECSXP of R_NilValue elements and
/// `HASHPRI` 0.
// R: envir.c:R_HashResize
pub(crate) unsafe fn rehash(old: Sexp, new: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract; every store keeps GNU R's counts.
    unsafe {
        for counter in 0..size(old) {
            let mut c = chain_at(old, counter);
            while c != nil {
                let h = index(c.tag(), new);
                let new_chain = chain_at(new, h);
                if new_chain == nil {
                    set_pri(new, pri(new) + 1);
                }
                let tmp = c;
                c = c.cdr();
                tmp.set_cdr(new_chain);
                new.set_elt(h, tmp);
            }
        }
    }
}

/// `R_HashDelete`: unlink `sym`'s cell from its chain; whether it was
/// there.
///
/// # Safety
/// `table` a live hash table, `sym` a live symbol, `unbound` R_UnboundValue.
// R: envir.c:R_HashDelete
pub(crate) unsafe fn delete(sym: Sexp, table: Sexp, nil: Sexp, unbound: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let idx = index(sym, table);
        let (list, found) = chain::remove(sym, chain_at(table, idx), nil, unbound);
        if found {
            if list == nil {
                set_pri(table, pri(table) - 1);
            }
            table.set_elt(idx, list);
        }
        found
    }
}

/// Every cell of the table in GNU R's listing order: chain by chain, each
/// from its head (`HashTableNames`, `HashTableValues`).
///
/// # Safety
/// `table` a live hash table; `f` changes no chain of it.
pub(crate) unsafe fn for_each_cell(table: Sexp, nil: Sexp, mut f: impl FnMut(Sexp)) {
    // SAFETY: the caller's contract.
    unsafe {
        for i in 0..size(table) {
            let mut c = chain_at(table, i);
            while c != nil {
                let next = c.cdr();
                f(c);
                c = next;
            }
        }
    }
}
