//! Symbols and their per-isolate cells (docs/g3.md §5.3).
//!
//! Symbols are the process's (H5): robigo-heap's arena makes each name's
//! symbol once, gives it a global id in the order the arena made them, and
//! stores GNU R's `R_Newhashpjw` of the name next to it
//! (`robigo_heap::OFF_SYMID`). A symbol never changes after it is made.
//! What it carries: its name, its hash, its id, and the bits its name gives
//! it ([`symbol_bits`]: GNU R's `SPECIAL_SYMBOL`, `DDVAL`, and no reference
//! counting on `.Last.value`).
//!
//! Everything an isolate binds to a symbol lives here, in tables indexed by
//! the id, never on the symbol:
//!
//! - the base binding: GNU R's `SYMVALUE` and the binding bits GNU R keeps
//!   on the symbol for it (active, locked, assignment pending);
//! - the symbol's entry of GNU R's `R_GlobalCache` (`envir.c:616-772`);
//! - its `INTERNAL`.
//!
//! The heap keeps the isolate's PRINTNAME of each symbol (`Core::printname`,
//! [`crate::Sym::name`]). The tables grow on demand, since another isolate
//! may have made a symbol this one has never seen.

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
            pub const ALL: [RSym; [$(RSym::$v),*].len()] = [$(RSym::$v),*];


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
    /// `srcfile` (`R_SrcfileSymbol`)
    Srcfile = b"srcfile",
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
    // (`.Last.value`'s NO_TRACKREFS comes with it: `symbol_bits`)
    RSym::ALL.map(|s| core.install(s.name()).expect("robigo: no room for the pre-installed symbols"))
}

/// The bits a new symbol is made with, from its name alone
/// (`robigo_heap::set_symbol_bits`, registered when an isolate is made):
/// GNU R's `SPECIAL_SYMBOL` for names.c's `Spec_name`, `DDVAL` for `..N`
/// (`mkSYMSXP`), and no reference counting on `.Last.value`'s binding
/// (`DISABLE_REFCNT(R_LastvalueSymbol)`). Nothing writes a symbol after it
/// is made (H5: symbols become process-shared and immutable).
// R: names.c:InitNames, names.c:mkSYMSXP, names.c:R_initAssignSymbols
pub(crate) fn symbol_bits(name: &[u8]) -> (u8, u16) {
    let mut gp = 0;
    if is_spec_name(name) {
        gp |= SPECIAL_SYMBOL;
    }
    if crate::fresh::is_dd_name(name) {
        gp |= robigo_heap::gp::sym::DDVAL;
    }
    let flags = if name == b".Last.value" { robigo_heap::flags::NO_TRACKREFS } else { 0 };
    (flags, gp)
}

/// Whether `name` is one of [`SPEC_NAME`]. The arena calls the symbol-bits
/// rule under its lock for every new symbol, so the common answer, no, is
/// one table load: [`SPEC_SHAPES`] has, per first byte, the lengths of the
/// special names that start with it. Only a name of such a shape is
/// compared with the list. Exact by construction (both come from
/// `SPEC_NAME`).
#[inline]
pub(crate) fn is_spec_name(name: &[u8]) -> bool {
    let (Some(&b), n @ 1..16) = (name.first(), name.len()) else {
        return false;
    };
    SPEC_SHAPES[b as usize] & (1 << n) != 0 && SPEC_NAME.iter().any(|s| s.as_bytes() == name)
}

/// Per first byte, a bit per length (below 16) of the [`SPEC_NAME`]s that
/// start with it.
const SPEC_SHAPES: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < SPEC_NAME.len() {
        let n = SPEC_NAME[i].as_bytes();
        assert!(!n.is_empty() && n.len() < 16, "a special name's length is a bit of SPEC_SHAPES");
        t[n[0] as usize] |= 1 << n.len();
        i += 1;
    }
    t
};

/// The cells a heap's first symbol cell makes: one per symbol id, made at
/// once instead of growing through the boot.
/// - Measured: a base boot (`--vanilla -e 1`, `ROBIGO_RT_BOOT=rdb`, #922's
///   head 2ca695862) makes 1,970 symbols, whose ids stay below 2,048. From
///   1,024 cells it grew twice, for 74,583 instructions.
/// - Cost: 2,048 × 24 bytes = 48 KiB per isolate, which a boot reached
///   anyway. Ids are the process's (the arena's), so a later isolate's
///   are as large as the first's.
const CELLS: usize = 2048;

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

/// Every symbol this isolate installed, in the order GNU R's symbol table
/// is walked (`envir.c:BuiltinNames`): chain `R_Newhashpjw(name) % HSIZE`
/// ascending, and within a chain newest first, since `install` prepends.
/// The ids are the order the process's arena made the symbols in, which is
/// this isolate's install order when it is the only one, so this is GNU
/// R's order whenever the symbols were installed in GNU R's order.
/// (`R_UnboundValue` and `R_MissingArg` are not installed, as in GNU R.)
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
///
/// The first growth makes [`CELLS`] cells at once.
#[derive(Default, Trace)]
pub(crate) struct SymCells {
    cells: Vec<SymCell>,
    /// GNU R's `INTERNAL(sym)`, by id: the `.Internal` function the symbol
    /// names (null for none). Kept apart from [`SymCell`], which variable
    /// lookups copy, since few symbols have one.
    internals: Vec<Sexp>,
}

impl SymCells {
    /// The base values and `.Internal`s, for the freeze's roots: not the
    /// global caches (`EnvState::trace_freeze_roots`).
    pub(crate) fn trace_freeze_values<V: robigo_heap::Visitor + ?Sized>(&mut self, v: &mut V) {
        for c in &mut self.cells {
            v.edge(&mut c.value);
        }
        for x in &mut self.internals {
            v.edge(x);
        }
    }

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
        let n = (i + 1).max(self.cells.len() * 2).max(CELLS);
        self.cells.resize(n, SymCell::EMPTY);
    }

    /// `INTERNAL(sym)`: null for none.
    pub(crate) fn internal(&self, id: SymId) -> Sexp {
        self.internals.get(id.0 as usize).copied().unwrap_or(Sexp::NULL)
    }

    /// `SET_INTERNAL(sym, x)`.
    pub(crate) fn set_internal(&mut self, id: SymId, x: Sexp) {
        let i = id.0 as usize;
        if i >= self.internals.len() {
            self.internals.resize(i + 1, Sexp::NULL);
        }
        // a freeze root (the boot heap's freeze): a frozen value replaced
        // leaves the frozen graph
        // SAFETY: the table's values are null or live.
        unsafe { robigo_heap::note_overwritten(self.internals[i]) };
        self.internals[i] = x;
    }

    /// Every `.Internal` function the table holds.
    pub(crate) fn internals(&self) -> impl Iterator<Item = Sexp> + '_ {
        self.internals.iter().copied().filter(|x| !x.is_null())
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

#[cfg(test)]
mod tests {
    use super::{SPEC_NAME, is_spec_name};

    /// The filter answers as the linear compare it replaced, over every
    /// special name and its near misses: the same first byte and length
    /// with another byte changed (each position, each byte value), and one
    /// byte longer or shorter.
    #[test]
    fn the_special_name_filter_is_the_linear_compare() {
        let linear = |name: &[u8]| SPEC_NAME.iter().any(|n| n.as_bytes() == name);
        let mut checked = 0;
        let mut check = |name: &[u8]| {
            assert_eq!(is_spec_name(name), linear(name), "{:?}", String::from_utf8_lossy(name));
            checked += 1;
        };
        for n in SPEC_NAME {
            let n = n.as_bytes();
            check(n);
            for i in 0..n.len() {
                for b in 0..=255u8 {
                    let mut m = n.to_vec();
                    m[i] = b;
                    check(&m);
                }
            }
            check(&n[..n.len() - 1]);
            check(&[n, b"x"].concat());
        }
        for other in
            [&b""[..], b"is.na", b"iff", b"function2", b"repeat_", b"[[<-.data.frame", &[b'i'; 15], &[b'i'; 16]]
        {
            check(other);
        }
        assert!(checked > SPEC_NAME.len() * 256);
    }
}
