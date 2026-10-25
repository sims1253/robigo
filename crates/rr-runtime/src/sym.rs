//! Interned symbols.
//!
//! Symbols are `u32` indices into a per-thread table; comparing two symbols
//! is an integer comparison. Frequently used symbols are pre-interned at fixed
//! indices so they can be named as constants.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

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
}

#[derive(Default)]
struct Interner {
    names: Vec<Rc<str>>,
    map: HashMap<Rc<str>, Sym>,
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
        let rc: Rc<str> = Rc::from(name);
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

    pub fn name(self) -> Rc<str> {
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
        if d > 0 {
            Some(d as usize)
        } else {
            None
        }
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
