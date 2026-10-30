//! Interned symbols.
//!
//! Symbols are `u32` indices into a per-thread table; comparing two symbols
//! is an integer comparison. Frequently used symbols are pre-interned at fixed
//! indices so they can be named as constants.

use crate::value::RString;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

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
        let sym = Sym(self.names.len() as u32);
        let dd = name.strip_prefix("..").and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
        self.ddvals.push(dd);
        self.names.push(rc.clone());
        self.map.insert(rc, sym);
        sym
    }
}

impl Sym {
    pub fn new(name: &str) -> Sym {
        INTERNER.with(|i| i.borrow_mut().intern(name))
    }

    /// The symbol named `name` if it has been interned, without interning
    /// it (new symbols shift the ids, and so the frame masks, of all later
    /// ones).
    pub fn lookup(name: &str) -> Option<Sym> {
        INTERNER.with(|i| i.borrow().map.get(name).copied())
    }

    /// The symbol `a.b` (an S3 method name), without allocating.
    pub fn dotted(a: &str, b: &str) -> Sym {
        thread_local! {
            static BUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        }
        BUF.with(|buf| {
            let mut buf = buf.borrow_mut();
            buf.clear();
            buf.push_str(a);
            buf.push('.');
            buf.push_str(b);
            Sym::new(&buf)
        })
    }

    pub fn name(self) -> RString {
        INTERNER.with(|i| i.borrow().names[self.0 as usize].clone())
    }

    pub fn with_name<R>(self, f: impl FnOnce(&str) -> R) -> R {
        INTERNER.with(|i| f(&i.borrow().names[self.0 as usize]))
    }

    pub fn is_empty(self) -> bool {
        self == s::EMPTY
    }

    /// `..1`, `..2`, ...: returns the 1-based index.
    #[inline]
    pub fn ddval(self) -> Option<usize> {
        let d = INTERNER.with(|i| i.borrow().ddvals[self.0 as usize]);
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
