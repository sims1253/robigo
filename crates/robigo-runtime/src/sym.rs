//! Interned symbols.
//!
//! Symbols carry a 31-bit table index and a `..n` flag; comparing two symbols
//! is an integer comparison. Frequently used symbols are pre-interned at fixed
//! indices so they can be named as constants.

use crate::value::RString;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

const DDVAL_FLAG: u32 = 1 << 31;

macro_rules! predefined {
    ($($name:ident = $text:expr),* $(,)?) => {
        const PREDEFINED: &[&str] = &[$($text),*];
        #[allow(non_upper_case_globals)]
        pub mod s {
            use super::Sym;
            predefined!(@consts 0u32; $($name),*);
        }
    };
    (@consts $n:expr; $name:ident $(, $rest:ident)*) => {
        pub const $name: Sym = Sym($n);
        predefined!(@consts $n + 1u32; $($rest),*);
    };
    (@consts $n:expr;) => {};
}

predefined! {
    EMPTY = "",
    DOTS = "...",
    NAMES = "names",
    DIM = "dim",
    DIMNAMES = "dimnames",
    CLASS = "class",
    LEVELS = "levels",
    ROW_NAMES = "row.names",
    TSP = "tsp",
    COMMENT = "comment",
    SRCREF = "srcref",
    TMP = "*tmp*",
    VALUE = "value",
    FUNCTION = "function",
    BRACE = "{",
    PAREN = "(",
    IF = "if",
    FOR = "for",
    WHILE = "while",
    REPEAT = "repeat",
    ASSIGN = "<-",
    SUPER_ASSIGN = "<<-",
    EQ_ASSIGN = "=",
    BRACKET = "[",
    BRACKET2 = "[[",
    DOLLAR = "$",
    AT = "@",
    TILDE = "~",
    QUOTE = "quote",
    DOT_GENERIC = ".Generic",
    DOT_CLASS = ".Class",
    DOT_METHOD = ".Method",
    DOT_GENERIC_CALL_ENV = ".GenericCallEnv",
    DOT_GENERIC_DEF_ENV = ".GenericDefEnv",
    DOT_GROUP = ".Group",
    DOT_DEFAULTS = ".defaults",
    DOT_S3_METHODS_TABLE = ".__S3MethodsTable__.",
    DOT_DATA = ".Data",
    DOT_ENVIRONMENT = ".Environment",
    DROP = "drop",
    EXACT = "exact",
    X = "x",
    NA_RM = "na.rm",
    DEFAULT = "default",
    MISSING = "missing",
    NAMESPACE_GET = "::",
    NAMESPACE_GET_INT = ":::",
    DOT_INTERNAL = ".Internal",
    PREVIOUS = "previous",
    CALL = "call",
    MESSAGE = "message",
    MINUS = "-",
    PLUS = "+",
    UNCLASS = "unclass",
    ELLIPSIS_ELT = "...elt",
    NAME = "name",
    DOT_PACKAGE_NAME = ".packageName",
    DOT_NAMESPACE = ".__NAMESPACE__.",
    SPEC = "spec",
    LOCAL = "local",
}

/// FxHash (as in rustc): fast for short strings such as symbol names.
#[derive(Default, Clone, Copy)]
pub struct StrHasher(u64);

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

impl StrHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(FX_K);
    }
}

impl std::hash::Hasher for StrHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type StrHash = std::hash::BuildHasherDefault<StrHasher>;

#[derive(Default)]
struct Interner {
    names: Vec<RString>,
    map: HashMap<RString, Sym, StrHash>,
    /// `..n` index of each symbol, 0 if it is not of that form.
    ddvals: Vec<u32>,
}

thread_local! {
    static INTERNER: RefCell<Interner> = RefCell::new({
        let mut i = Interner::default();
        for name in PREDEFINED {
            i.intern(name);
        }
        i
    });
}

impl Interner {
    fn intern(&mut self, name: &str) -> Sym {
        if let Some(s) = self.map.get(name) {
            return *s;
        }
        let rc = RString::from(name);
        assert!(self.names.len() < DDVAL_FLAG as usize, "too many interned symbols");
        let dd = name.strip_prefix("..").and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
        let sym = Sym(self.names.len() as u32 | if dd != 0 { DDVAL_FLAG } else { 0 });
        self.ddvals.push(dd);
        self.names.push(rc.clone());
        self.map.insert(rc, sym);
        sym
    }
}

impl Sym {
    /// The table index, excluding the cached `..n` flag.
    #[inline(always)]
    pub fn index(self) -> usize {
        (self.0 & !DDVAL_FLAG) as usize
    }

    /// Whether evaluation reads an element of `...`, rather than a binding.
    #[inline(always)]
    pub fn has_ddval(self) -> bool {
        self.0 & DDVAL_FLAG != 0
    }

    pub fn new(name: &str) -> Sym {
        INTERNER.with(|i| i.borrow_mut().intern(name))
    }

    /// R: names.c:installTrChar — a name a *runtime* caller installs (an
    /// assignment target, `as.symbol`, ...). Unlike the parser's symbols,
    /// the empty string and names over MAXIDSIZE (10000 bytes) are
    /// rejected; the message is R's.
    pub fn install_tr(name: &str) -> std::result::Result<Sym, &'static str> {
        if name.is_empty() {
            return Err("attempt to use zero-length variable name");
        }
        if name.len() > 10000 {
            return Err("variable names are limited to 10000 bytes");
        }
        Ok(Sym::new(name))
    }

    /// The symbol named `name` if it has been interned, without interning
    /// it (new symbols shift the ids, and so the frame masks, of all later
    /// ones).
    pub fn lookup(name: &str) -> Option<Sym> {
        INTERNER.with(|i| i.borrow().map.get(name).copied())
    }

    /// The symbol `a.b` (an S3 method name), without allocating.
    ///
    /// S3 dispatch asks for the same few names over and over, so the
    /// answers are memoized in a small table indexed by a hash of the two
    /// strings. A slot is trusted only after comparing its symbol's name
    /// with `a.b` byte for byte, so a collision just recomputes.
    pub fn dotted(a: &str, b: &str) -> Sym {
        const SLOTS: usize = 128;
        struct Memo {
            hits: [Option<(RString, Sym)>; SLOTS],
            buf: String,
        }
        thread_local! {
            static MEMO: RefCell<Memo> =
                const { RefCell::new(Memo { hits: [const { None }; SLOTS], buf: String::new() }) };
        }
        // FxHash-style over both strings; the class name varies most.
        let mut h: u64 = a.len() as u64;
        for bytes in [a.as_bytes(), b.as_bytes()] {
            let mut chunks = bytes.chunks_exact(8);
            for c in &mut chunks {
                h = (h.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap()))
                    .wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
            }
            for &c in chunks.remainder() {
                h = (h.rotate_left(5) ^ c as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
            }
        }
        let slot = (h >> 57) as usize % SLOTS;
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            if let Some((name, sym)) = &m.hits[slot] {
                let n = name.as_bytes();
                if n.len() == a.len() + 1 + b.len()
                    && n[a.len()] == b'.'
                    && &n[..a.len()] == a.as_bytes()
                    && &n[a.len() + 1..] == b.as_bytes()
                {
                    return *sym;
                }
            }
            let m = &mut *m;
            m.buf.clear();
            m.buf.push_str(a);
            m.buf.push('.');
            m.buf.push_str(b);
            let sym = Sym::new(&m.buf);
            m.hits[slot] = Some((sym.name(), sym));
            sym
        })
    }

    pub fn name(self) -> RString {
        INTERNER.with(|i| i.borrow().names[self.index()].clone())
    }

    pub fn with_name<R>(self, f: impl FnOnce(&str) -> R) -> R {
        INTERNER.with(|i| f(&i.borrow().names[self.index()]))
    }

    pub fn is_empty(self) -> bool {
        self == s::EMPTY
    }

    /// `..1`, `..2`, ...: returns the 1-based index.
    #[inline]
    pub fn ddval(self) -> Option<usize> {
        if !self.has_ddval() {
            return None;
        }
        let d = INTERNER.with(|i| i.borrow().ddvals[self.index()]);
        if d > 0 { Some(d as usize) } else { None }
    }
}

impl fmt::Debug for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_name(|n| write!(f, "`{}`", n))
    }
}

impl fmt::Display for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_name(|n| f.write_str(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_names_share_the_interned_string() {
        for text in ["", "ordinary", "é", "..12"] {
            let sym = Sym::new(text);
            let a = sym.name();
            let b = sym.name();
            assert_eq!(&*a, text);
            assert!(RString::ptr_eq(&a, &b));
            assert_eq!(Sym::lookup(text), Some(sym));
        }
    }
}

#[cfg(test)]
mod dotdot_tests {
    use super::*;

    #[test]
    fn dotdot_flag_preserves_symbol_identity_and_table_indices() {
        for (name, value) in [("..1", 1), ("..12", 12), ("..0002", 2)] {
            let symbol = Sym::new(name);
            assert!(symbol.has_ddval());
            assert_eq!(symbol.ddval(), Some(value));
            assert_eq!(&*symbol.name(), name);
            assert_eq!(Sym::lookup(name), Some(symbol));
        }
        for name in ["x", "...", "..0", "..name"] {
            let symbol = Sym::new(name);
            assert!(!symbol.has_ddval());
            // Ordinary symbol checks must not borrow the interner.
            INTERNER.with(|interner| {
                let _held = interner.borrow_mut();
                assert_eq!(symbol.ddval(), None);
            });
        }
    }
}
