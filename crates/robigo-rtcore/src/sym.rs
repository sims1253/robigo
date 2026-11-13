//! Symbols and their per-isolate cells (docs/g3.md §5.3): the one module
//! behind which M2's process-shared symbol arena swaps in.
//!
//! In M1 symbols are per heap (foreman's answer 1 to m1-plan §F): the heap
//! interns them, gives each a dense id in the order it made them and stores
//! GNU R's `R_Newhashpjw` of the name next to it (`robigo_heap::OFF_SYMID`).
//! Everything an isolate binds to a symbol lives here, in a table indexed by
//! that id, never in the symbol itself:
//!
//! - the base binding: GNU R's `SYMVALUE` and the two binding bits GNU R
//!   keeps on the symbol for it (active, locked);
//! - the symbol's entry of GNU R's `R_GlobalCache` (`envir.c:616-772`).
//!
//! What stays on the symbol is what never changes after it is made: its
//! name, its hash, its id, and GNU R's `SPECIAL_SYMBOL` bit (names.c's
//! `Spec_name`, set once per heap). M2 replaces [`id_of`], [`hash_of`] and
//! [`for_each_installed`] with the arena's arithmetic and walk; the tables
//! below already grow on demand, since another isolate may install a symbol
//! this one has never seen.

#![allow(unsafe_code)]

use robigo_heap::{Core, Sexp, Trace};

/// GNU R's symbol table size (`Defn.h:HSIZE`): the base environment lists
/// its bindings in the order of a walk over that many chains.
pub(crate) const HSIZE: u32 = 49157;

/// GNU R's binding bits (`Defn.h`, "use the same bits (15 and 14) in symbols
/// and bindings"): on a binding cell's gp, and in a [`SymCell`] for a base
/// binding.
pub(crate) const ACTIVE_BINDING: u16 = 1 << 15;
pub(crate) const BINDING_LOCK: u16 = 1 << 14;
/// `ASSIGNMENT_PENDING_MASK` (Defn.h): a complex assignment to this
/// binding is in progress, and the next store keeps the old value's count.
pub(crate) const ASSIGNMENT_PENDING: u16 = 1 << 11;
/// `SPECIAL_SYMBOL_MASK` on a symbol's gp (and, on an environment,
/// `NO_SPECIAL_SYMBOLS`).
pub(crate) const SPECIAL_SYMBOL: u16 = 1 << 12;

/// names.c's `Spec_name`: the symbols marked `SPECIAL_SYMBOL` when the
/// symbol table is made. Closure frames that never bound one are skipped
/// when a function named by one is looked up (`findFun3`).
// R: names.c:Spec_name
pub(crate) const SPEC_NAME: &[&str] = &[
    "if", "while", "repeat", "for", "break", "next", "return", "function", "(", "{", "+", "-", "*", "/", "^", "%%",
    "%/%", "%*%", ":", "::", ":::", "?", "|>", "~", "@", "=>", "==", "!=", "<", ">", "<=", ">=", "&", "|", "&&", "||",
    "!", "<-", "<<-", "=", "$", "[", "[[", "$<-", "[<-", "[[<-",
];

/// A symbol's id: dense per heap in M1 (the order the heap made its
/// symbols), global in M2 (the arena's arithmetic).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SymId(pub u32);

/// The id of symbol `s`.
///
/// # Safety
/// `s` is a live SYMSXP.
#[inline(always)]
pub(crate) unsafe fn id_of(s: Sexp) -> SymId {
    // SAFETY: the caller's contract.
    SymId(unsafe { s.sym_id() })
}

/// GNU R's `R_Newhashpjw` of `s`'s name (what `HASHVALUE(PRINTNAME(s))`
/// holds once `HASHASH` is set).
///
/// # Safety
/// `s` is a live SYMSXP.
#[inline(always)]
pub(crate) unsafe fn hash_of(s: Sexp) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { s.sym_hash() }
}

/// GNU R's `IS_SPECIAL_SYMBOL`.
///
/// # Safety
/// `s` is a live SYMSXP.
#[inline(always)]
pub(crate) unsafe fn is_special(s: Sexp) -> bool {
    // SAFETY: the caller's contract.
    unsafe { s.gp() & SPECIAL_SYMBOL != 0 }
}

/// Every installed symbol in the order GNU R's symbol table is walked
/// (`envir.c:BuiltinNames`): chain `R_Newhashpjw(name) % HSIZE` ascending,
/// and within a chain newest first, since `install` prepends. The ids are
/// the heap's install order, so this is GNU R's order whenever the symbols
/// were installed in GNU R's order. (`R_UnboundValue` and `R_MissingArg`
/// are not installed, as in GNU R.)
pub(crate) fn for_each_installed(core: &Core, mut f: impl FnMut(Sexp)) {
    let mut syms: Vec<(u32, u32, Sexp)> = core
        .symbols()
        // SAFETY: the heap's symbol table holds live (immortal) symbols.
        .map(|s| unsafe { (hash_of(s) % HSIZE, id_of(s).0, s) })
        .collect();
    syms.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    for (_, _, s) in syms {
        f(s);
    }
}

/// One symbol's per-isolate state.
#[derive(Clone, Copy, Debug, Trace)]
pub(crate) struct SymCell {
    /// The base binding's value (GNU R's `SYMVALUE`); null when unbound,
    /// the function of an active binding.
    pub(crate) value: Sexp,
    /// The symbol's `R_GlobalCache` entry: null (none, or flushed), the
    /// symbol itself (the visible binding from the global environment is
    /// the base one: GNU R's `BASE_SYM_CACHED`), or a binding cell of a
    /// global frame.
    pub(crate) cache: Sexp,
    /// [`ACTIVE_BINDING`], [`BINDING_LOCK`] and [`ASSIGNMENT_PENDING`] of
    /// the base binding (GNU R keeps them on the symbol's gp).
    #[trace(skip = "binding bits, no object")]
    pub(crate) bits: u16,
}

impl SymCell {
    const EMPTY: SymCell = SymCell { value: Sexp::NULL, cache: Sexp::NULL, bits: 0 };
}

/// The isolate's symbol cells, indexed by [`SymId`]. A symbol without a
/// cell yet is unbound in base, uncached and has no bits.
#[derive(Default, Trace)]
pub(crate) struct SymCells {
    cells: Vec<SymCell>,
}

impl SymCells {
    #[inline(always)]
    pub(crate) fn get(&self, id: SymId) -> SymCell {
        self.cells.get(id.0 as usize).copied().unwrap_or(SymCell::EMPTY)
    }

    #[inline]
    pub(crate) fn get_mut(&mut self, id: SymId) -> &mut SymCell {
        let i = id.0 as usize;
        if i >= self.cells.len() {
            self.grow(i);
        }
        &mut self.cells[i]
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self, i: usize) {
        let n = (i + 1).max(self.cells.len() * 2).max(1024);
        self.cells.resize(n, SymCell::EMPTY);
    }

    /// The cells that exist (ids at or past it have none).
    pub(crate) fn len(&self) -> usize {
        self.cells.len()
    }

    /// Cell `i < len()`.
    pub(crate) fn at(&self, i: usize) -> SymCell {
        self.cells[i]
    }
}
