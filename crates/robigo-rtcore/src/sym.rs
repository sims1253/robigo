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
//! name, its hash, its id, GNU R's `SPECIAL_SYMBOL` bit (names.c's
//! `Spec_name`, set once per heap) and its TRACKREFS (off for
//! `.Last.value` alone, [`install_rsyms`]). M2 replaces [`id_of`], [`hash_of`] and
//! [`for_each_installed`] with the arena's arithmetic and walk; the tables
//! below already grow on demand, since another isolate may install a symbol
//! this one has never seen.

#![allow(unsafe_code)]

use robigo_heap::{Core, Sexp, Trace};

/// The enum of the pre-installed symbols, its [`RSym::ALL`] and names, from
/// one list.
macro_rules! rsyms {
    ($($(#[$doc:meta])* $v:ident = $name:literal,)*) => {
        /// GNU R's pre-installed symbols that builtins name (`R_NamesSymbol`,
        /// `R_DimSymbol`, ...: names.c's `InitNames`), read with
        /// [`crate::Cx::sym`]; the C API's symbol shortcuts are among them
        /// (`crate::capi`). They are interned when the isolate is made and
        /// symbols are immortal, so reading one allocates nothing and cannot
        /// fail (no `NeedGc`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum RSym {
            $($(#[$doc])* $v,)*
        }

        impl RSym {
            /// Every symbol, in the order of the table (an index is its
            /// discriminant).
            pub(crate) const ALL: [RSym; [$(RSym::$v),*].len()] = [$(RSym::$v),*];


            /// The symbol's name.
            pub const fn name(self) -> &'static [u8] {
                match self {
                    $(RSym::$v => $name,)*
                }
            }
        }

        // (`capi::symbols` and `Cx::sym` index the interned table with the
        // discriminant)
        const _: () = {
            let mut i = 0;
            while i < RSym::ALL.len() {
                assert!(RSym::ALL[i] as usize == i, "RSym: an index is not its discriminant");
                i += 1;
            }
        };
    };
}

rsyms! {
    /// `names` (`R_NamesSymbol`)
    Names = b"names",
    /// `dim` (`R_DimSymbol`)
    Dim = b"dim",
    /// `dimnames` (`R_DimNamesSymbol`)
    DimNames = b"dimnames",
    /// `class` (`R_ClassSymbol`)
    Class = b"class",
    /// `tsp` (`R_TspSymbol`)
    Tsp = b"tsp",
    /// `levels` (`R_LevelsSymbol`)
    Levels = b"levels",
    /// `row.names` (`R_RowNamesSymbol`)
    RowNames = b"row.names",
    /// `comment` (`R_CommentSymbol`)
    Comment = b"comment",
    /// `srcref` (`R_SrcrefSymbol`)
    Srcref = b"srcref",
    /// `...` (`R_DotsSymbol`)
    Dots = b"...",
    /// `as.character` (`R_AsCharacterSymbol`)
    AsCharacter = b"as.character",
    /// `@` (`R_AtsignSymbol`)
    Atsign = b"@",
    /// `base` (`R_BaseSymbol, R_baseSymbol`)
    Base = b"base",
    /// `{` (`R_BraceSymbol`)
    Brace = b"{",
    /// `[[` (`R_Bracket2Symbol`)
    Bracket2 = b"[[",
    /// `[` (`R_BracketSymbol`)
    Bracket = b"[",
    /// `.Device` (`R_DeviceSymbol`)
    Device = b".Device",
    /// `$` (`R_DollarSymbol`)
    Dollar = b"$",
    /// `::` (`R_DoubleColonSymbol`)
    DoubleColon = b"::",
    /// `drop` (`R_DropSymbol`)
    Drop = b"drop",
    /// `eval` (`R_EvalSymbol`)
    Eval = b"eval",
    /// `function` (`R_FunctionSymbol`)
    Function = b"function",
    /// `.Last.value` (`R_LastvalueSymbol`)
    LastValue = b".Last.value",
    /// `mode` (`R_ModeSymbol`)
    Mode = b"mode",
    /// `na.rm` (`R_NaRmSymbol`)
    NaRm = b"na.rm",
    /// `name` (`R_NameSymbol`)
    Name = b"name",
    /// `.__NAMESPACE__.` (`R_NamespaceEnvSymbol`)
    NamespaceEnv = b".__NAMESPACE__.",
    /// `package` (`R_PackageSymbol`)
    Package = b"package",
    /// `previous` (`R_PreviousSymbol`)
    Previous = b"previous",
    /// `quote` (`R_QuoteSymbol`)
    Quote = b"quote",
    /// `.Random.seed` (`R_SeedsSymbol`)
    Seeds = b".Random.seed",
    /// `sort.list` (`R_SortListSymbol`)
    SortList = b"sort.list",
    /// `source` (`R_SourceSymbol`)
    Source = b"source",
    /// `spec` (`R_SpecSymbol`)
    Spec = b"spec",
    /// `:::` (`R_TripleColonSymbol`)
    TripleColon = b":::",
    /// `.defined` (`R_dot_defined`)
    DotDefined = b".defined",
    /// `.Method` (`R_dot_Method`)
    DotMethod = b".Method",
    /// `.packageName` (`R_dot_packageName`)
    DotPackageName = b".packageName",
    /// `.target` (`R_dot_target`)
    DotTarget = b".target",
    /// `.Generic` (`R_dot_Generic`)
    DotGeneric = b".Generic",
}

/// [`RSym::ALL`] interned in `core`, in that order. Panics if the new heap
/// has no room for them.
///
/// `.Last.value` is made to count no reference on its base binding, as
/// GNU R's `DISABLE_REFCNT(R_LastvalueSymbol)`: the value of the last
/// top-level expression is not duplicated at its next change.
// R: names.c:R_initAssignSymbols
pub(crate) fn install_rsyms(core: &mut Core) -> [Sexp; RSym::ALL.len()] {
    let syms = RSym::ALL.map(|s| core.install(s.name()).expect("robigo: no room for the pre-installed symbols"));
    // SAFETY: a symbol just interned (live and immortal); a header flag,
    // set before any binding of it is stored.
    unsafe { syms[RSym::LastValue as usize].set_flag(robigo_heap::flags::NO_TRACKREFS, true) };
    syms
}

/// GNU R's symbol table size (`Defn.h:HSIZE`): the base environment lists
/// its bindings in the order of a walk over that many chains.
pub(crate) const HSIZE: u32 = 49157;

/// GNU R's binding bits (`Defn.h`, "use the same bits (15 and 14) in symbols
/// and bindings"): on a binding cell's gp, and in a [`SymCell`] for a base
/// binding.
/// `ASSIGNMENT_PENDING_MASK` (Defn.h): a complex assignment to this
/// binding is in progress, and the next store keeps the old value's count.
pub(crate) use robigo_heap::gp::cell::{ACTIVE_BINDING, ASSIGNMENT_PENDING, BINDING_LOCK};
/// `SPECIAL_SYMBOL_MASK` on a symbol's gp (and, on an environment,
/// `NO_SPECIAL_SYMBOLS`).
pub(crate) use robigo_heap::gp::sym::SPECIAL_SYMBOL;

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
