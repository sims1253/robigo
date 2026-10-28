//! Deparsing: R objects back to source text. Port of GNU R's deparse.c.

use crate::format::{NumFormat, encode_element_for_deparse};
use crate::sym::{Sym, s};
use crate::value::*;

pub const DEFAULT_CUTOFF: usize = 60;
pub const MAX_CUTOFF: usize = 500;

// deparse options (from Defn.h)
pub const KEEPINTEGER: u32 = 1;
pub const QUOTEEXPRESSIONS: u32 = 2;
pub const SHOWATTRIBUTES: u32 = 4;
pub const USESOURCE: u32 = 8;
pub const WARNINCOMPLETE: u32 = 16;
pub const DELAYPROMISES: u32 = 32;
pub const KEEPNA: u32 = 64;
pub const S_COMPAT: u32 = 128;
pub const HEXNUMERIC: u32 = 256;
pub const DIGITS17: u32 = 512;
pub const NICE_NAMES: u32 = 1024;
pub const SIMPLEDEPARSE: u32 = 0;
pub const DEFAULTDEPARSE: u32 = 1089; // KEEPINTEGER | KEEPNA | NICE_NAMES

const SIMPLE_OPTS: u32 = !QUOTEEXPRESSIONS & !SHOWATTRIBUTES & !DELAYPROMISES;
const SHOW_ATTR_OR_NMS: u32 = SHOWATTRIBUTES | NICE_NAMES;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PP {
    Assign,
    Assign2,
    Binary,
    Binary2,
    Break,
    Curly,
    For,
    Funcall,
    Function,
    If,
    Next,
    Paren,
    Return,
    Subass,
    Subset,
    While,
    Unary,
    Dollar,
    Repeat,
}

const PREC_FN: u8 = 0;
const PREC_EQ: u8 = 1;
const PREC_LEFT: u8 = 2;
const PREC_TILDE: u8 = 4;
const PREC_OR: u8 = 5;
const PREC_AND: u8 = 6;
const PREC_NOT: u8 = 7;
const PREC_COMPARE: u8 = 8;
const PREC_SUM: u8 = 9;
const PREC_PROD: u8 = 10;
const PREC_PERCENT: u8 = 11;
const PREC_COLON: u8 = 12;
const PREC_SIGN: u8 = 13;
const PREC_POWER: u8 = 14;
const PREC_SUBSET: u8 = 15;
const PREC_DOLLAR: u8 = 16;
const PREC_NS: u8 = 17;

#[derive(Clone, Copy, Debug)]
struct PPInfo {
    kind: PP,
    prec: u8,
    rightassoc: bool,
}

/// PPINFO of builtins with special syntax (names.c). Everything else that
/// is a builtin deparses as a function call.
fn ppinfo(name: &str) -> Option<PPInfo> {
    let (kind, prec, ra) = match name {
        "if" => (PP::If, PREC_FN, true),
        "while" => (PP::While, PREC_FN, false),
        "for" => (PP::For, PREC_FN, false),
        "repeat" => (PP::Repeat, PREC_FN, false),
        "break" => (PP::Break, PREC_FN, false),
        "next" => (PP::Next, PREC_FN, false),
        "return" => (PP::Return, PREC_FN, false),
        "function" => (PP::Function, PREC_FN, false),
        "<-" => (PP::Assign, PREC_LEFT, true),
        "=" => (PP::Assign, PREC_EQ, true),
        "<<-" => (PP::Assign2, PREC_LEFT, true),
        "{" => (PP::Curly, PREC_FN, false),
        "(" => (PP::Paren, PREC_FN, false),
        "[" | "[[" => (PP::Subset, PREC_SUBSET, false),
        "$" | "@" => (PP::Dollar, PREC_DOLLAR, false),
        "[<-" | "[[<-" | "$<-" | "@<-" => (PP::Subass, PREC_LEFT, true),
        "+" | "-" => (PP::Binary, PREC_SUM, false),
        "*" => (PP::Binary, PREC_PROD, false),
        "/" => (PP::Binary2, PREC_PROD, false),
        "^" => (PP::Binary2, PREC_POWER, true),
        "%%" | "%/%" => (PP::Binary2, PREC_PERCENT, false),
        "%*%" => (PP::Binary, PREC_PERCENT, false),
        "==" | "!=" | "<" | "<=" | ">=" | ">" => (PP::Binary, PREC_COMPARE, false),
        "&" | "&&" => (PP::Binary, PREC_AND, false),
        "|" | "||" => (PP::Binary, PREC_OR, false),
        "!" => (PP::Unary, PREC_NOT, false),
        ":" => (PP::Binary2, PREC_COLON, false),
        "~" => (PP::Binary, PREC_TILDE, false),
        "::" | ":::" => (PP::Binary2, PREC_NS, false),
        _ => return None,
    };
    Some(PPInfo { kind, prec, rightassoc: ra })
}

/// Names of builtins that deparse as plain function calls; used to decide
/// whether a symbol head is "a builtin" for parenthesization.
fn is_builtin_funcall(name: &str) -> bool {
    // Any builtin not in ppinfo() is PP_FUNCALL; for deparsing purposes only
    // the distinction between operators and calls matters.
    ppinfo(name).is_none()
}

fn is_user_binop(name: &str) -> bool {
    // R: deparse.c — %wrapped% operators that are not primitives
    // (PRIMSPEC) print with spaces; the primitive %% and %/% keep their
    // PP_BINARY2 entries and print without any (names.c).
    name.len() >= 2 && name.starts_with('%') && name.ends_with('%') && ppinfo(name).is_none()
}

const RESERVED: &[&str] = &[
    "if",
    "else",
    "repeat",
    "while",
    "function",
    "for",
    "next",
    "break",
    "TRUE",
    "FALSE",
    "NULL",
    "Inf",
    "NaN",
    "NA",
    "NA_integer_",
    "NA_real_",
    "NA_character_",
    "NA_complex_",
    "in",
];

/// R's `isValidName`: syntactic, non-reserved name.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(c0) = chars.next() else { return false };
    if c0 == '.' {
        if let Some(c1) = name.chars().nth(1) {
            if c1.is_ascii_digit() {
                return false;
            }
        }
    } else if !c0.is_alphabetic() {
        return false;
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_') {
        return false;
    }
    if name == "..." {
        return true;
    }
    !RESERVED.contains(&name)
}

fn quotify(name: &str, quote: char) -> String {
    if is_valid_name(name) || name.is_empty() {
        name.to_string()
    } else {
        let mut out = Vec::new();
        robigo_syntax::dump::encode_string(name.as_bytes(), quote as u8, &mut out);
        String::from_utf8_lossy(&out).into_owned()
    }
}

struct D {
    lines: Vec<String>,
    buf: String,
    len: usize,
    incurly: i32,
    inlist: i32,
    startline: bool,
    indent: i32,
    left: u8,
    cutoff: usize,
    backtick: bool,
    opts: u32,
    maxlines: usize,
    active: bool,
    fnarg: bool,
}

impl D {
    fn new(cutoff: usize, backtick: bool, opts: u32, maxlines: usize) -> D {
        D {
            lines: Vec::new(),
            buf: String::new(),
            len: 0,
            incurly: 0,
            inlist: 0,
            startline: true,
            indent: 0,
            left: 0,
            cutoff,
            backtick,
            opts,
            maxlines,
            active: true,
            fnarg: false,
        }
    }

    fn print(&mut self, s: &str) {
        if self.startline {
            self.startline = false;
            for i in 1..=self.indent {
                self.buf.push_str(if i <= 4 { "    " } else { "  " });
                self.len += if i <= 4 { 4 } else { 2 };
            }
        }
        self.buf.push_str(s);
        self.len += s.len();
    }

    fn writeline(&mut self) {
        if self.lines.len() < self.maxlines {
            self.lines.push(std::mem::take(&mut self.buf));
        }
        self.buf.clear();
        if self.lines.len() >= self.maxlines {
            self.active = false;
        }
        self.len = 0;
        self.startline = true;
    }

    fn linebreak(&mut self, lbreak: &mut bool) {
        if self.len > self.cutoff {
            if !*lbreak {
                *lbreak = true;
                self.indent += 1;
            }
            self.writeline();
        }
    }

    fn curlyahead(v: &Value) -> bool {
        matches!(v, Value::Lang(l) if l.fun.as_sym() == Some(s::BRACE))
    }

    fn needsparens(&self, mainop: PPInfo, arg: &Value, left: bool, deep_left: u8) -> bool {
        match arg {
            Value::Lang(l) => {
                if let Value::Sym(sym) = &l.fun {
                    let name = sym.name();
                    if let Some(mut arginfo) = ppinfo(&name) {
                        if matches!(arginfo.kind, PP::Binary | PP::Binary2) {
                            match l.args.len() {
                                1 => {
                                    if arginfo.prec == PREC_SUM {
                                        arginfo.prec = PREC_SIGN;
                                    }
                                    arginfo.kind = PP::Unary;
                                }
                                2 => {}
                                _ => return false,
                            }
                        }
                        match arginfo.kind {
                            PP::Subset => {
                                if matches!(mainop.kind, PP::Dollar | PP::Subset) && mainop.prec > arginfo.prec {
                                    return false;
                                }
                                // fall through to binary handling
                                self.binary_needs(mainop, arginfo, left)
                            }
                            PP::Binary | PP::Binary2 => {
                                if mainop.prec == PREC_COMPARE && arginfo.prec == PREC_COMPARE {
                                    return true;
                                }
                                self.binary_needs(mainop, arginfo, left)
                            }
                            PP::Assign | PP::Assign2 | PP::Dollar => self.binary_needs(mainop, arginfo, left),
                            PP::Unary => (left && mainop.prec > arginfo.prec) || (deep_left > arginfo.prec),
                            PP::For | PP::If | PP::While | PP::Repeat => left || deep_left > 0,
                            _ => false,
                        }
                    } else if is_user_binop(&name) {
                        mainop.prec > PREC_PERCENT || (mainop.prec == PREC_PERCENT && left == mainop.rightassoc)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Value::Cplx(v) if v.data.len() == 1 => {
                mainop.prec > PREC_SUM || (mainop.prec == PREC_SUM && left == mainop.rightassoc)
            }
            _ => false,
        }
    }

    fn binary_needs(&self, mainop: PPInfo, arginfo: PPInfo, left: bool) -> bool {
        mainop.prec > arginfo.prec || (mainop.prec == arginfo.prec && left == mainop.rightassoc)
    }

    /// attr1(): decide how attributes are shown; prints "structure(" if needed.
    fn attr1(&mut self, v: &Value) -> AttrType {
        let nice_names = self.opts & NICE_NAMES != 0;
        let show_attr = self.opts & SHOWATTRIBUTES != 0;
        let nm = v.get_attr(s::NAMES);
        let has_names = nm.is_some();
        let mut attr = AttrType::Unknown;
        if let Some(nm) = &nm {
            let ok_names = nice_names && usable_nice_names(nm, v.is_atomic());
            if !ok_names {
                attr = if show_attr { AttrType::StrucNmsA } else { AttrType::OkNames };
            }
        }
        if attr == AttrType::Unknown {
            if let Some(a) = v.attrs() {
                for (tag, _) in &a.0 {
                    if has_names && *tag == s::NAMES {
                        continue;
                    }
                    if show_attr && *tag != s::SRCREF {
                        attr = AttrType::StrucAttr;
                        break;
                    }
                }
            }
        }
        if attr == AttrType::Unknown {
            attr = if has_names { AttrType::OkNames } else { AttrType::Simple };
        }
        if attr >= AttrType::StrucAttr {
            self.print("structure(");
        }
        attr
    }

    fn attr2(&mut self, v: &Value, not_names: bool) {
        if let Some(a) = v.attrs().cloned() {
            for (tag, val) in &a.0 {
                if *tag == s::SRCREF || (*tag == s::NAMES && not_names) {
                    continue;
                }
                self.print(", ");
                let name = tag.name();
                match &*name {
                    "dim" | "dimnames" | "names" | "tsp" | "levels" => self.print(&name),
                    _ => {
                        if is_valid_name(&name) {
                            self.print(&name);
                        } else {
                            self.print("\"");
                            self.print(&name);
                            self.print("\"");
                        }
                    }
                }
                self.print(" = ");
                let fnarg = self.fnarg;
                self.fnarg = true;
                self.deparse(val);
                self.fnarg = fnarg;
            }
        }
        self.print(")");
    }

    fn deparse(&mut self, v: &Value) {
        let fnarg = self.fnarg;
        self.fnarg = false;
        let prev_left = self.left;
        self.left = 0;
        if !self.active {
            return;
        }
        let opts_in = self.opts;
        match v {
            Value::Null => self.print("NULL"),
            Value::Sym(sym) => {
                let name = sym.name();
                let doquote = opts_in & QUOTEEXPRESSIONS != 0 && !name.is_empty();
                if doquote {
                    self.print("quote(");
                }
                if opts_in & S_COMPAT != 0 {
                    self.print(&quotify(&name, '"'));
                } else if self.backtick {
                    self.print(&quotify(&name, '`'));
                } else {
                    self.print(&name);
                }
                if doquote {
                    self.print(")");
                }
            }
            Value::Builtin(b) => {
                self.print(".Primitive(\"");
                self.print(b.name);
                self.print("\")");
            }
            Value::Promise(p) => {
                if let Some(val) = p.value() {
                    self.deparse(&val);
                } else {
                    let e = p.expr.clone();
                    self.deparse(&e);
                }
            }
            Value::Closure(c) => {
                let attr = if opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(v) } else { AttrType::Simple };
                self.opts &= SIMPLE_OPTS & !USESOURCE;
                self.print("function (");
                self.args2buff(&c.formals, false, true);
                self.print(") ");
                self.writeline();
                self.deparse(&c.body);
                self.opts = opts_in;
                if attr >= AttrType::StrucAttr {
                    self.attr2(v, attr == AttrType::StrucAttr);
                }
            }
            Value::Env(_) => self.print("<environment>"),
            Value::List(lv) => {
                let attr = if opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(v) } else { AttrType::Simple };
                self.print("list(");
                self.opts = opts_in;
                let items = lv.data.clone();
                self.vec2buff(&items, v, attr == AttrType::OkNames || attr == AttrType::StrucAttr);
                self.opts |= NICE_NAMES;
                self.print(")");
                if attr >= AttrType::StrucAttr {
                    self.attr2(v, attr == AttrType::StrucAttr);
                }
                self.opts = opts_in;
            }
            Value::Expr(ev) => {
                let attr = if opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(v) } else { AttrType::Simple };
                if ev.data.is_empty() {
                    self.print("expression()");
                } else {
                    let loc = self.opts;
                    self.print("expression(");
                    self.opts &= SIMPLE_OPTS;
                    let items = ev.data.clone();
                    self.vec2buff(&items, v, attr == AttrType::OkNames || attr == AttrType::StrucAttr);
                    self.opts = loc;
                    self.print(")");
                }
                if attr >= AttrType::StrucAttr {
                    self.attr2(v, attr == AttrType::StrucAttr);
                }
                self.opts = opts_in;
            }
            Value::Pairlist(pl) => {
                let attr = if opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(v) } else { AttrType::Simple };
                let missing = pl.items.iter().any(|a| a.value.is_missing_arg());
                self.print(if missing { "as.pairlist(alist(" } else { "pairlist(" });
                self.inlist += 1;
                let n = pl.items.len();
                for (i, a) in pl.items.iter().enumerate() {
                    if let Some(t) = a.tag {
                        self.opts = SIMPLEDEPARSE;
                        self.deparse(&Value::Sym(t));
                        self.opts = opts_in;
                        self.print(" = ");
                    }
                    self.deparse(&a.value);
                    if i + 1 < n {
                        self.print(", ");
                    }
                }
                self.print(if missing { "))" } else { ")" });
                self.inlist -= 1;
                if attr >= AttrType::StrucAttr {
                    self.attr2(v, attr == AttrType::StrucAttr);
                }
            }
            Value::Dots(_) => self.print("<...>"),
            Value::ExtPtr(_) => self.print("<pointer: 0x0>"),
            Value::Obj(_) => self.print("<object>"),
            Value::Lang(l) => self.deparse_lang(l, opts_in, fnarg, prev_left),
            Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_) => {
                self.vector2buff(v)
            }
        }
        self.left = prev_left;
    }

    fn deparse_lang(&mut self, l: &Lang, opts_in: u32, fnarg: bool, prev_left: u8) {
        let op = &l.fun;
        let maybe_quote = opts_in & QUOTEEXPRESSIONS != 0;
        let mut doquote = false;
        if maybe_quote {
            doquote = op.as_sym().is_none_or(|s| *s.name() != *"~");
            if doquote {
                self.print("quote(");
                self.opts &= SIMPLE_OPTS;
            } else {
                self.opts &= !QUOTEEXPRESSIONS;
            }
        }
        let args = &l.args;
        if let Value::Sym(sym) = op {
            let name = sym.name();
            let userbinop = is_user_binop(&name);
            let info = if userbinop {
                if args.iter().all(|a| a.tag.is_none()) {
                    Some(PPInfo { kind: PP::Binary2, prec: PREC_PERCENT, rightassoc: false })
                } else {
                    Some(PPInfo { kind: PP::Funcall, prec: PREC_FN, rightassoc: false })
                }
            } else {
                ppinfo(&name)
            };
            if let Some(mut fop) = info {
                match fop.kind {
                    PP::Binary => match args.len() {
                        1 => {
                            fop.kind = PP::Unary;
                            if fop.prec == PREC_SUM {
                                fop.prec = PREC_SIGN;
                            }
                        }
                        2 => {}
                        _ => fop.kind = PP::Funcall,
                    },
                    PP::Binary2 => {
                        if args.len() != 2 {
                            fop.kind = PP::Funcall;
                        } else if userbinop {
                            fop.kind = PP::Binary;
                        }
                    }
                    PP::Dollar => {
                        if args.len() != 2 {
                            fop.kind = PP::Funcall;
                        } else {
                            let rhs = &args[1].value;
                            let ok = matches!(rhs, Value::Sym(_))
                                || matches!(rhs, Value::Str(v) if v.data.len() == 1 && v.data[0].is_some());
                            if !ok {
                                fop.kind = PP::Funcall;
                            }
                        }
                    }
                    _ => {}
                }
                // Calls with tagged args to operators deparse as calls.
                self.deparse_op(&name, fop, args, fnarg, prev_left);
            } else {
                // Regular function call.
                let _ = is_builtin_funcall(&name);
                if opts_in & S_COMPAT != 0 {
                    self.print(&quotify(&name, '\''));
                } else {
                    self.print(&quotify(&name, '`'));
                }
                self.print("(");
                self.inlist += 1;
                self.args2buff(args, false, false);
                self.inlist -= 1;
                self.print(")");
            }
        } else {
            if parenthesize_caller(op) {
                self.print("(");
                self.deparse(op);
                self.print(")");
            } else {
                self.deparse(op);
            }
            self.print("(");
            self.args2buff(args, false, false);
            self.print(")");
        }
        if maybe_quote {
            self.opts = opts_in;
            if doquote {
                self.print(")");
            }
        }
    }

    fn arg(args: &[Arg], i: usize) -> Value {
        args.get(i).map(|a| a.value.clone()).unwrap_or(Value::Null)
    }

    fn deparse_op(&mut self, name: &str, fop: PPInfo, args: &[Arg], fnarg: bool, prev_left: u8) {
        let mut lbreak = false;
        match fop.kind {
            PP::If => {
                self.print("if (");
                self.deparse(&Self::arg(args, 0));
                self.print(") ");
                let mut lookahead = false;
                if self.incurly > 0 && self.inlist == 0 {
                    lookahead = Self::curlyahead(&Self::arg(args, 1));
                    if !lookahead {
                        self.writeline();
                        self.indent += 1;
                    }
                }
                if args.len() > 2 {
                    self.deparse(&Self::arg(args, 1));
                    if self.incurly > 0 && self.inlist == 0 {
                        self.writeline();
                        if !lookahead {
                            self.indent -= 1;
                        }
                    } else {
                        self.print(" ");
                    }
                    self.print("else ");
                    self.deparse(&Self::arg(args, 2));
                } else {
                    self.deparse(&Self::arg(args, 1));
                    if self.incurly > 0 && !lookahead && self.inlist == 0 {
                        self.indent -= 1;
                    }
                }
            }
            PP::While => {
                self.print("while (");
                self.deparse(&Self::arg(args, 0));
                self.print(") ");
                self.deparse(&Self::arg(args, 1));
            }
            PP::For => {
                self.print("for (");
                self.deparse(&Self::arg(args, 0));
                self.print(" in ");
                self.deparse(&Self::arg(args, 1));
                self.print(") ");
                self.deparse(&Self::arg(args, 2));
            }
            PP::Repeat => {
                self.print("repeat ");
                self.deparse(&Self::arg(args, 0));
            }
            PP::Curly => {
                self.print("{");
                self.incurly += 1;
                self.indent += 1;
                self.writeline();
                for a in args {
                    self.deparse(&a.value);
                    self.writeline();
                }
                self.indent -= 1;
                self.print("}");
                self.incurly -= 1;
            }
            PP::Paren => {
                self.print("(");
                self.deparse(&Self::arg(args, 0));
                self.print(")");
            }
            PP::Subset => {
                let first = Self::arg(args, 0);
                let parens = self.needsparens(fop, &first, true, prev_left);
                if parens {
                    self.print("(");
                }
                self.deparse(&first);
                if parens {
                    self.print(")");
                }
                let single = name == "[";
                self.print(if single { "[" } else { "[[" });
                self.args2buff(&args[1.min(args.len())..], false, false);
                self.print(if single { "]" } else { "]]" });
            }
            PP::Funcall | PP::Return => {
                if self.backtick {
                    self.print(&quotify(name, '`'));
                } else {
                    self.print(&quotify(name, '"'));
                }
                self.print("(");
                self.inlist += 1;
                self.args2buff(args, false, false);
                self.inlist -= 1;
                self.print(")");
            }
            PP::Function => {
                self.print(name);
                self.print("(");
                if let Some(Value::Pairlist(p)) = args.first().map(|a| &a.value) {
                    let items = p.items.clone();
                    self.args2buff(&items, false, true);
                }
                self.print(") ");
                self.deparse(&Self::arg(args, 1));
            }
            PP::Assign | PP::Assign2 => {
                let outerparens = fnarg && name == "=";
                if outerparens {
                    self.print("(");
                }
                let lhs = Self::arg(args, 0);
                let parens = self.needsparens(fop, &lhs, true, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { fop.prec };
                self.deparse(&lhs);
                if parens {
                    self.print(")");
                }
                self.print(" ");
                self.print(name);
                self.print(" ");
                let rhs = Self::arg(args, 1);
                let parens = self.needsparens(fop, &rhs, false, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { prev_left };
                self.deparse(&rhs);
                if parens {
                    self.print(")");
                }
                if outerparens {
                    self.print(")");
                }
                self.left = 0;
            }
            PP::Dollar => {
                let lhs = Self::arg(args, 0);
                let parens = self.needsparens(fop, &lhs, true, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { fop.prec };
                self.deparse(&lhs);
                if parens {
                    self.print(")");
                }
                self.print(name);
                let rhs = Self::arg(args, 1);
                match &rhs {
                    Value::Str(sv) if sv.data.len() == 1 && sv.data[0].as_deref().is_some_and(is_valid_name) => {
                        let s = sv.data[0].clone().unwrap();
                        self.print(&s);
                    }
                    _ => {
                        let parens = self.needsparens(fop, &rhs, false, prev_left);
                        if parens {
                            self.print("(");
                        }
                        self.left = if parens { 0 } else { prev_left };
                        self.deparse(&rhs);
                        if parens {
                            self.print(")");
                        }
                    }
                }
                self.left = 0;
            }
            PP::Binary => {
                let lhs = Self::arg(args, 0);
                let parens = self.needsparens(fop, &lhs, true, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { fop.prec };
                self.deparse(&lhs);
                if parens {
                    self.print(")");
                }
                self.print(" ");
                self.print(name);
                self.print(" ");
                self.linebreak(&mut lbreak);
                let rhs = Self::arg(args, 1);
                let parens = self.needsparens(fop, &rhs, false, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { prev_left };
                self.deparse(&rhs);
                if parens {
                    self.print(")");
                }
                if lbreak {
                    self.indent -= 1;
                }
                self.left = 0;
            }
            PP::Binary2 => {
                let lhs = Self::arg(args, 0);
                let parens = self.needsparens(fop, &lhs, true, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { fop.prec };
                self.deparse(&lhs);
                if parens {
                    self.print(")");
                }
                self.print(name);
                let rhs = Self::arg(args, 1);
                let parens = self.needsparens(fop, &rhs, false, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { prev_left };
                self.deparse(&rhs);
                if parens {
                    self.print(")");
                }
                self.left = 0;
            }
            PP::Unary => {
                self.print(name);
                let a = Self::arg(args, 0);
                let parens = self.needsparens(fop, &a, false, prev_left);
                if parens {
                    self.print("(");
                }
                self.left = if parens { 0 } else { prev_left };
                self.deparse(&a);
                if parens {
                    self.print(")");
                }
                self.left = 0;
            }
            PP::Break => self.print("break"),
            PP::Next => self.print("next"),
            PP::Subass => {
                self.print("`");
                self.print(name);
                self.print("`(");
                self.args2buff(args, false, false);
                self.print(")");
            }
        }
    }

    fn vec2buff(&mut self, items: &[Value], v: &Value, do_names: bool) {
        let mut lbreak = false;
        let nv = if do_names { v.get_attr(s::NAMES) } else { None };
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.print(", ");
            }
            self.linebreak(&mut lbreak);
            if let Some(nv) = &nv {
                self.deparse_name(nv, i);
            }
            self.deparse(item);
        }
        if lbreak {
            self.indent -= 1;
        }
    }

    fn deparse_name(&mut self, nv: &Value, i: usize) {
        if let Value::Str(sv) = nv {
            if let Some(Some(n)) = sv.data.get(i) {
                if !n.is_empty() {
                    if is_valid_name(n) {
                        self.print(n);
                    } else if self.backtick {
                        self.print("`");
                        self.print(n);
                        self.print("`");
                    } else {
                        self.print("\"");
                        self.print(n);
                        self.print("\"");
                    }
                    self.print(" = ");
                }
            }
        }
    }

    fn args2buff(&mut self, args: &[Arg], _lineb: bool, formals: bool) {
        let mut lbreak = false;
        let n = args.len();
        for (i, a) in args.iter().enumerate() {
            if let Some(tag) = a.tag {
                let name = tag.name();
                if tag == s::DOTS {
                    self.print(&name);
                } else if self.backtick {
                    self.print(&quotify(&name, '`'));
                } else {
                    self.print(&quotify(&name, '"'));
                }
                if formals {
                    if !a.value.is_missing_arg() {
                        self.print(" = ");
                        self.fnarg = true;
                        self.deparse(&a.value);
                    }
                } else {
                    self.print(" = ");
                    if !a.value.is_missing_arg() {
                        self.fnarg = true;
                        self.deparse(&a.value);
                    }
                }
            } else {
                self.fnarg = true;
                self.deparse(&a.value);
            }
            if i + 1 < n {
                self.print(", ");
                self.linebreak(&mut lbreak);
            }
        }
        if lbreak {
            self.indent -= 1;
        }
    }

    fn vector2buff(&mut self, v: &Value) {
        let tlen = v.len();
        let opts_in = self.opts;
        let int_seq = match v {
            Value::Int(iv) if tlen > 1 => {
                let d = iv.data[1] as f64 - iv.data[0] as f64;
                iv.data[0] != NA_INTEGER
                    && iv.data[1] != NA_INTEGER
                    && d.abs() == 1.0
                    && iv.data.windows(2).all(|w| w[1] != NA_INTEGER && (w[1] as f64 - w[0] as f64) == d)
            }
            _ => false,
        };
        let mut do_names = opts_in & SHOW_ATTR_OR_NMS != 0;
        let nv = if do_names { v.get_attr(s::NAMES) } else { None };
        if nv.is_none() {
            do_names = false;
        }
        let str_names = do_names && (int_seq || tlen == 0);
        if str_names {
            self.opts &= !NICE_NAMES;
        }
        let attr = if opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(v) } else { AttrType::Simple };
        if do_names {
            do_names = attr == AttrType::OkNames || attr == AttrType::StrucAttr;
        }
        let need_c = tlen > 1 || do_names;
        if tlen == 0 {
            self.print(match v {
                Value::Lgl(_) => "logical(0)",
                Value::Int(_) => "integer(0)",
                Value::Real(_) => "numeric(0)",
                Value::Cplx(_) => "complex(0)",
                Value::Str(_) => "character(0)",
                _ => "raw(0)",
            });
        } else if let Value::Int(iv) = v {
            if int_seq {
                let a = encode_element_for_deparse(v, 0);
                let b = encode_element_for_deparse(v, tlen - 1);
                self.print(&a);
                self.print(":");
                self.print(&b);
            } else {
                let add_l = self.opts & KEEPINTEGER != 0 && self.opts & S_COMPAT == 0;
                let mut all_na = self.opts & KEEPNA != 0 || add_l;
                if iv.data.iter().any(|x| *x != NA_INTEGER) {
                    all_na = false;
                }
                if need_c {
                    self.print("c(");
                }
                for i in 0..tlen {
                    if do_names {
                        self.deparse_name(nv.as_ref().unwrap(), i);
                    }
                    if all_na && iv.data[i] == NA_INTEGER {
                        self.print("NA_integer_");
                    } else {
                        let e = encode_element_for_deparse(v, i);
                        self.print(&e);
                        if add_l && iv.data[i] != NA_INTEGER {
                            self.print("L");
                        }
                    }
                    if i + 1 < tlen {
                        self.print(", ");
                    }
                    if tlen > 1 && self.len > self.cutoff {
                        self.writeline();
                    }
                    if !self.active {
                        break;
                    }
                }
                if need_c {
                    self.print(")");
                }
            }
        } else {
            let mut all_na = self.opts & KEEPNA != 0;
            let mut surround = false;
            match v {
                Value::Real(rv) if all_na => {
                    if rv.data.iter().any(|x| !is_na_real(*x)) {
                        all_na = false;
                    }
                }
                Value::Cplx(cv) if all_na => {
                    if cv.data.iter().any(|x| !is_na_real(x.re) && !is_na_real(x.im)) {
                        all_na = false;
                    }
                }
                Value::Str(sv) if all_na => {
                    if sv.data.iter().any(|x| x.is_some()) {
                        all_na = false;
                    }
                }
                Value::Raw(_) => {
                    self.print("as.raw(");
                    surround = true;
                }
                _ => {}
            }
            if need_c {
                self.print("c(");
            }
            for i in 0..tlen {
                if do_names {
                    self.deparse_name(nv.as_ref().unwrap(), i);
                }
                let strp: String = match v {
                    Value::Real(rv) if all_na && is_na_real(rv.data[i]) => "NA_real_".into(),
                    Value::Cplx(cv) if is_na_real(cv.data[i].re) && is_na_real(cv.data[i].im) => {
                        if all_na {
                            "NA_complex_".into()
                        } else {
                            encode_element_for_deparse(v, i)
                        }
                    }
                    Value::Str(sv) if all_na && sv.data[i].is_none() => "NA_character_".into(),
                    Value::Raw(rv) => format!("0x{:02x}", rv.data[i]),
                    // R: deparse.c:vector2buff -- hexNumeric and digits17.
                    Value::Real(rv) if self.opts & HEXNUMERIC != 0 && rv.data[i].is_finite() => {
                        crate::format::encode_hex_real(rv.data[i])
                    }
                    Value::Real(rv) if self.opts & DIGITS17 != 0 && rv.data[i].is_finite() => {
                        robigo_syntax::cfmt::fmt_g(rv.data[i], 17, false, false)
                    }
                    Value::Cplx(cv)
                        if self.opts & HEXNUMERIC != 0 && cv.data[i].re.is_finite() && cv.data[i].im.is_finite() =>
                    {
                        crate::format::encode_hex_complex(cv.data[i])
                    }
                    Value::Cplx(cv)
                        if self.opts & DIGITS17 != 0 && cv.data[i].re.is_finite() && cv.data[i].im.is_finite() =>
                    {
                        format!(
                            "{}{}i",
                            robigo_syntax::cfmt::fmt_g(cv.data[i].re, 17, false, false),
                            crate::format::encode_digits17_signed(cv.data[i].im)
                        )
                    }
                    _ => encode_element_for_deparse(v, i),
                };
                self.print(&strp);
                if i + 1 < tlen {
                    self.print(", ");
                }
                if tlen > 1 && self.len > self.cutoff {
                    self.writeline();
                }
                if !self.active {
                    break;
                }
            }
            if need_c {
                self.print(")");
            }
            if surround {
                self.print(")");
            }
        }
        if attr >= AttrType::StrucAttr {
            self.attr2(v, attr == AttrType::StrucAttr);
        }
        if str_names {
            self.opts = opts_in;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum AttrType {
    Unknown,
    Simple,
    OkNames,
    StrucAttr,
    StrucNmsA,
}

fn usable_nice_names(nm: &Value, is_atomic: bool) -> bool {
    if let Value::Str(sv) = nm {
        let mut all_0 = true;
        for x in &sv.data {
            match x {
                None => return false,
                Some(s) => {
                    if is_atomic && (&**s == "recursive" || &**s == "use.names") {
                        return false;
                    }
                    if !s.is_empty() {
                        all_0 = false;
                    }
                }
            }
        }
        return !all_0;
    }
    true
}

fn parenthesize_caller(op: &Value) -> bool {
    match op {
        Value::Lang(l) => match &l.fun {
            Value::Sym(sym) => {
                let name = sym.name();
                if is_user_binop(&name) {
                    return true;
                }
                match ppinfo(&name) {
                    Some(info) => {
                        !(info.prec >= PREC_SUBSET || matches!(info.kind, PP::Funcall | PP::Paren | PP::Curly))
                    }
                    None => false,
                }
            }
            _ => true,
        },
        Value::Closure(_) => true,
        _ => false,
    }
}

/// `deparse(x, width.cutoff, backtick, opts, nlines)`.
pub fn deparse_lines(v: &Value, cutoff: usize, backtick: bool, opts: u32, nlines: Option<usize>) -> Vec<String> {
    let mut d = D::new(cutoff, backtick, opts, nlines.unwrap_or(usize::MAX));
    d.deparse(v);
    d.writeline();
    d.lines
}

/// Default deparse with R's defaults (`deparse(x)`).
pub fn deparse(v: &Value) -> Vec<String> {
    deparse_lines(v, DEFAULT_CUTOFF, matches!(v, Value::Lang(_) | Value::Sym(_)), DEFAULTDEPARSE, None)
}

/// `deparse1line`: all lines concatenated.
pub fn deparse_one_line(v: &Value) -> String {
    let lines = deparse_lines(v, MAX_CUTOFF, true, SIMPLEDEPARSE, None);
    join_lines(&lines)
}

pub fn join_lines(lines: &[String]) -> String {
    // R: deparse.c:deparse1line_ex — lines are concatenated with "\n"
    // between them (strcat(buf, "\n") for every line but the last).
    lines.join("\n")
}

/// `deparse1s`: first line only, used for error and warning calls.
pub fn deparse_for_message(call: &Value) -> String {
    let lines = deparse_lines(call, DEFAULT_CUTOFF, true, DEFAULTDEPARSE, Some(1));
    lines.into_iter().next().unwrap_or_default()
}

pub fn is_syntactic(name: &str) -> bool {
    is_valid_name(name)
}

pub fn sym_text(sym: Sym) -> String {
    quotify(&sym.name(), '`')
}

/// Formatting digits used when deparsing doubles (DBL_DIG).
pub const DEPARSE_DIGITS: NumFormat = NumFormat { digits: 15 };
