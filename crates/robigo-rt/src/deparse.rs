//! Deparsing: an object back to R source text, ported from GNU R 4.6.2's
//! `deparse.c` (`deparse1WithCutoff`, `deparse2buff`, `args2buff`,
//! `vec2buff`, `vector2buff`, `attr1`/`attr2`, `needsparens`,
//! `parenthesizeCaller`, `linebreak`, `writeline`).
//!
//! Deparsing reads objects and writes text: it runs under `&Cx`, allocates
//! nothing on the heap and returns the lines, which the caller makes a
//! character vector of (`do_deparse`), prints (`PrintLanguage`) or puts in
//! a message (`deparse1s`, the calls of errors and warnings). The few
//! places where GNU R's deparse runs code are handled without it:
//!
//! - a promise (`case PROMSXP`, `eval(s, R_EmptyEnv)`): its value when it
//!   is forced; an unforced promise says "not implemented";
//! - `SYMVALUE(op)` of a call's function is read from base without
//!   forcing; GNU R forces a lazy base binding only to see whether it is
//!   the closure of `::` or `:::`, which are primitives in R 4.6.2, so the
//!   forced value never changes the text;
//! - S4 objects (`new("cls", ...)` through methods), srcrefs
//!   (`as.character(srcref)`) and the `hexNumeric`, `digits17` and `S_compatible`
//!   encodings of numbers say "not implemented".
//!
//! `R_BrowseLines` is 0 and `getOption("deparse.max.lines")` unset (no
//! `options()` yet), so `deparse1m` is `deparse1`.

use crate::prims;
use robigo_heap::types::*;
use robigo_rtbuiltins::registry::{PpInfo, PpKind, Prec, pp_info};
use robigo_rtcore::{Char, Cx, Rcomplex, Ref};
use robigo_text::num::{
    encode_complex, encode_integer, encode_logical, encode_raw, encode_real0, format_complex, format_integer,
    format_logical, format_real,
};
use robigo_text::string::{Justify, encode_string, format_string};
use robigo_text::{PrintParams, Str, is_na};

// The deparse options (Defn.h).
pub const KEEPINTEGER: i32 = 1;
pub const QUOTEEXPRESSIONS: i32 = 2;
pub const SHOWATTRIBUTES: i32 = 4;
pub const USESOURCE: i32 = 8;
pub const WARNINCOMPLETE: i32 = 16;
pub const DELAYPROMISES: i32 = 32;
pub const KEEPNA: i32 = 64;
pub const S_COMPAT: i32 = 128;
pub const HEXNUMERIC: i32 = 256;
pub const DIGITS17: i32 = 512;
pub const NICE_NAMES: i32 = 1024;
pub const SIMPLEDEPARSE: i32 = 0;
/// KEEPINTEGER | KEEPNA | NICE_NAMES, used for calls.
pub const DEFAULTDEPARSE: i32 = 1089;

/// `MIN_Cutoff`, `DEFAULT_Cutoff`, `MAX_Cutoff`: deparse's line widths.
pub const MIN_CUTOFF: i32 = 20;
pub const DEFAULT_CUTOFF: i32 = 60;
pub const MAX_CUTOFF: i32 = 500;

/// keep KEEPINTEGER | USESOURCE | KEEPNA | S_COMPAT, also WARNINCOMPLETE
const SIMPLE_OPTS: i32 = !QUOTEEXPRESSIONS & !SHOWATTRIBUTES & !DELAYPROMISES;
const SHOW_ATTR_OR_NMS: i32 = SHOWATTRIBUTES | NICE_NAMES;

/// What a deparse made: its lines (native encoding) and whether it is
/// "sourceable" (`deparse may be incomplete` otherwise, under
/// `WARNINCOMPLETE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deparsed {
    pub lines: Vec<Vec<u8>>,
    pub sourceable: bool,
}

/// Why a deparse stopped: the message of the error to raise (GNU R's, or
/// robigo's "not implemented" for what is not ported).
pub type Fail = String;

type DResult = Result<(), Fail>;

/// `deparse1WithCutoff(call, abbrev, cutoff, backtick, opts, nlines)`:
/// `nlines > 0` keeps at most that many lines, otherwise all (nlines is
/// -1 from R, 0 from C callers; `R_BrowseLines` is 0).
// R: deparse.c:deparse1WithCutoff
pub fn deparse1_with_cutoff(
    cx: &Cx,
    call: Ref<'_>,
    abbrev: bool,
    cutoff: i32,
    backtick: bool,
    opts: i32,
    nlines: i32,
) -> Result<Deparsed, Fail> {
    let maxlines = if nlines > 0 { nlines as usize } else { usize::MAX };
    let mut d = State {
        cx,
        lines: Vec::new(),
        linenumber: 0,
        len: 0,
        incurly: 0,
        inlist: 0,
        startline: true,
        indent: 0,
        left: 0,
        buffer: Vec::new(),
        cutoff: cutoff as usize,
        backtick,
        opts,
        sourceable: true,
        maxlines,
        active: true,
        fnarg: false,
        // PrintDefaults(), then R_print.digits = DBL_DIG
        pp: PrintParams { digits: 15, locale: robigo_rtbuiltins::text::ctype(cx.locale()), ..PrintParams::DEFAULT },
    };
    // deparse2(): GNU R runs it once to count the lines and once to fill
    // them; one pass makes the same text
    d.deparse2buff(call)?;
    d.writeline();
    let mut lines = d.lines;
    if abbrev {
        let first = lines.first().cloned().unwrap_or_default();
        let mut data: Vec<u8> = first.iter().copied().take(10).collect();
        if first.len() > 10 {
            data.extend_from_slice(b"...");
        }
        lines = vec![data];
    }
    Ok(Deparsed { lines, sourceable: d.sourceable })
}

/// `deparse1(call, abbrev, opts)`: every line, the default cutoff,
/// backticks.
// R: deparse.c:deparse1, deparse1m
pub fn deparse1(cx: &Cx, call: Ref<'_>, abbrev: bool, opts: i32) -> Result<Deparsed, Fail> {
    deparse1_with_cutoff(cx, call, abbrev, DEFAULT_CUTOFF, true, opts, 0)
}

/// `deparse1w(call, abbrev, opts)`: for printing language objects, at
/// `R_print.cutoff`.
// R: deparse.c:deparse1w
pub fn deparse1w(cx: &Cx, call: Ref<'_>, abbrev: bool, opts: i32, cutoff: i32) -> Result<Deparsed, Fail> {
    deparse1_with_cutoff(cx, call, abbrev, cutoff, true, opts, -1)
}

/// `deparse1s(call)`: the first line, for the calls of errors and warnings.
// R: deparse.c:deparse1s
pub fn deparse1s(cx: &Cx, call: Ref<'_>) -> Result<Vec<u8>, Fail> {
    let d = deparse1_with_cutoff(cx, call, false, DEFAULT_CUTOFF, true, DEFAULTDEPARSE, 1)?;
    Ok(d.lines.into_iter().next().unwrap_or_default())
}

/// `deparse1line_ex(call, abbrev, opts)`: all lines joined by newlines,
/// at `MAX_Cutoff`.
// R: deparse.c:deparse1line_ex, deparse1line
pub fn deparse1line(cx: &Cx, call: Ref<'_>, abbrev: bool, opts: i32) -> Result<Vec<u8>, Fail> {
    let d = deparse1_with_cutoff(cx, call, abbrev, MAX_CUTOFF, true, opts, -1)?;
    Ok(d.lines.join(&b'\n'))
}

/// `LocalParseData`.
struct State<'g> {
    cx: &'g Cx,
    lines: Vec<Vec<u8>>,
    linenumber: usize,
    len: usize,
    incurly: i32,
    inlist: i32,
    startline: bool,
    indent: i32,
    /// the precedence of the operator whose left operand is being
    /// deparsed (`d->left`), 0 elsewhere
    left: u8,
    buffer: Vec<u8>,
    cutoff: usize,
    backtick: bool,
    opts: i32,
    sourceable: bool,
    maxlines: usize,
    active: bool,
    /// a function argument: parenthesize `=` as an assignment
    fnarg: bool,
    pp: PrintParams<'static>,
}

/// `attr_type`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Attr {
    Unknown,
    Simple,
    /// no structure(*); names written as (n1 = v1, ..)
    OkNames,
    /// structure(*, <attr> = *, ..) for non-names only
    StrucAttr,
    /// structure(*, <attr> = *, ..) for names, too
    StrucNmsA,
}

fn sym_name<'g>(x: Ref<'g>) -> Char<'g> {
    x.as_sym().expect("robigo: a symbol").name()
}

fn sym_is(x: Ref<'_>, name: &[u8]) -> bool {
    x.as_sym().is_some_and(|s| s.name().bytes() == name)
}

/// `isValidName` of a native string.
fn is_valid_name(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok_and(robigo_syntax::is_valid_name)
}

/// `isUserBinop(s)`: a symbol `%...%`.
// R: Rinlinedfuns.h:isUserBinop
fn is_user_binop(s: Ref<'_>) -> bool {
    s.as_sym().is_some_and(|s| {
        let b = s.name().bytes();
        b.len() >= 2 && b[0] == b'%' && b[b.len() - 1] == b'%'
    })
}

/// `PPINFO(SYMVALUE(op))` when `op`'s base binding is a primitive.
fn sym_prim(cx: &Cx, op: Ref<'_>) -> Option<(PpInfo, &'static str)> {
    let sym = op.as_sym()?;
    let base = cx.base_env();
    let rho = base.view();
    let robigo_rtcore::View::Env(rho) = rho else { return None };
    match cx.frame_binding(rho, sym) {
        robigo_rtcore::Lookup::Bound(b) => match b.get(cx) {
            robigo_rtcore::BindingValue::Value(v) => {
                let name = prims::prim_name(cx, v)?;
                Some((pp_info(name), name))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `curlyahead(s)`: a call (or pairlist) whose function is `{`.
// R: deparse.c:curlyahead
fn curlyahead(s: Ref<'_>) -> bool {
    matches!(s.sexptype(), LISTSXP | LANGSXP | NILSXP) && s.as_cons().is_some_and(|c| sym_is(c.car(), b"{"))
}

/// The cells of a pairlist or call.
fn cells<'g>(s: Ref<'g>) -> impl Iterator<Item = robigo_rtcore::Cons<'g>> {
    let mut at = s.as_cons();
    std::iter::from_fn(move || {
        let c = at?;
        at = c.cdr().as_cons();
        Some(c)
    })
}

/// `length()` of a pairlist or call.
fn plen(s: Ref<'_>) -> usize {
    cells(s).count()
}

/// The `n`-th car of a pairlist (`CAR`, `CADR`, `CADDR`).
fn nth<'g>(cx: &'g Cx, s: Ref<'g>, n: usize) -> Ref<'g> {
    cells(s).nth(n).map(|c| c.car()).unwrap_or(cx.nil())
}

/// `needsparens(mainop, arg, left, deepLeft)`: whether `arg`, an operand
/// of `mainop` (on its left when `left`), needs parentheses.
// R: deparse.c:needsparens
fn needsparens(cx: &Cx, mainop: PpInfo, arg: Ref<'_>, left: bool, deep_left: u8) -> bool {
    let mp = mainop.prec as u8;
    if arg.sexptype() == LANGSXP {
        let head = arg.as_cons().unwrap().car();
        if head.sexptype() == SYMSXP {
            if let Some((mut arginfo, _)) = sym_prim(cx, head) {
                // Not all binary ops are binary!
                if matches!(arginfo.kind, PpKind::Binary | PpKind::Binary2) {
                    match plen(arg.as_cons().unwrap().cdr()) {
                        1 => {
                            // binary +/- precedence upgraded as unary
                            if arginfo.prec == Prec::Sum {
                                arginfo.prec = Prec::Sign;
                            }
                            arginfo.kind = PpKind::Unary;
                        }
                        2 => {}
                        _ => return false,
                    }
                }
                let ap = arginfo.prec as u8;
                let binary_like = |cmp: bool| {
                    if cmp && mainop.prec == Prec::Compare && arginfo.prec == Prec::Compare {
                        return true; // a < b < c is not legal syntax
                    }
                    mp > ap || (mp == ap && left == mainop.right_assoc)
                };
                return match arginfo.kind {
                    PpKind::Subset => {
                        if matches!(mainop.kind, PpKind::Dollar | PpKind::Subset) && mp > ap {
                            return false;
                        }
                        binary_like(true)
                    }
                    PpKind::Binary | PpKind::Binary2 => binary_like(true),
                    PpKind::Assign | PpKind::Assign2 | PpKind::Dollar => binary_like(false),
                    PpKind::Unary => (left && mp > ap) || (deep_left != 0 && deep_left > ap),
                    PpKind::For | PpKind::If | PpKind::While | PpKind::Repeat => left || deep_left != 0,
                    _ => false,
                };
            } else if is_user_binop(head) {
                let pc = Prec::Percent as u8;
                if mp > pc || (mp == pc && left == mainop.right_assoc) {
                    return true;
                }
            }
        }
    } else if arg.sexptype() == CPLXSXP && arg.len() == Some(1) {
        let ps = Prec::Sum as u8;
        if mp > ps || (mp == ps && left == mainop.right_assoc) {
            return true;
        }
    }
    false
}

/// `parenthesizeCaller(s)`: whether the function of a call needs
/// parentheses (`(f+g)(z)`, `(function(x) 1)(x)`; not `x$f(z)`, `x[n](z)`).
// R: deparse.c:parenthesizeCaller
fn parenthesize_caller(cx: &Cx, s: Ref<'_>) -> bool {
    if s.sexptype() == LANGSXP {
        let op = s.as_cons().unwrap().car();
        if op.sexptype() == SYMSXP {
            if is_user_binop(op) {
                return true; // %foo%
            }
            if let Some((info, _)) = sym_prim(cx, op) {
                return !(info.prec >= Prec::Subset
                    || matches!(info.kind, PpKind::Funcall | PpKind::Paren | PpKind::Curly));
            }
            false // regular function call
        } else {
            true // something strange, like (1)(x)
        }
    } else {
        s.sexptype() == CLOSXP
    }
}

/// `getAttrib(x, R_NamesSymbol)`: the names attribute, a pairlist's or a
/// call's tags (None when it has no tag), or a one-dimensional array's
/// first dimnames.
// R: attrib.c:getAttrib
enum Names<'g> {
    Attr(Ref<'g>),
    Tags(Vec<Char<'g>>),
}

impl<'g> Names<'g> {
    fn get(cx: &'g Cx, x: Ref<'g>) -> Option<Names<'g>> {
        if matches!(x.sexptype(), LISTSXP | LANGSXP | DOTSXP) {
            let mut any = false;
            let mut v = Vec::new();
            for c in cells(x) {
                let t = c.tag();
                if let Some(s) = t.as_sym() {
                    any = true;
                    v.push(s.name());
                } else {
                    v.push(cx.blank_string());
                }
            }
            return any.then_some(Names::Tags(v));
        }
        let attrib = |n: &[u8]| robigo_rtbuiltins::attrib::get_value(x, n);
        if robigo_rtbuiltins::attrib::is_vector(x)
            && let Some(dim) = attrib(b"dim")
            && dim.len() == Some(1)
        {
            // isOneDimensionalArray
            if let Some(dn) = attrib(b"dimnames")
                && let robigo_rtcore::View::List(l) = dn.view()
            {
                let first = l.get(0);
                return (first.sexptype() != NILSXP).then_some(Names::Attr(first));
            }
            return None;
        }
        attrib(b"names").filter(|n| n.sexptype() != NILSXP).map(Names::Attr)
    }

    /// The `i`-th name (`NA_STRING` included).
    fn at(&self, i: usize) -> Option<Char<'g>> {
        match self {
            Names::Tags(v) => v.get(i).copied(),
            Names::Attr(a) => {
                let s = a.as_vector::<Char<'g>>()?;
                (i < s.len()).then(|| s.get(i))
            }
        }
    }

    fn strings(&self, cx: &'g Cx) -> Option<Vec<Char<'g>>> {
        match self {
            Names::Tags(v) => Some(v.clone()),
            Names::Attr(a) => {
                let _ = cx;
                let s = a.as_vector::<Char<'g>>()?;
                Some((0..s.len()).map(|i| s.get(i)).collect())
            }
        }
    }
}

impl<'g> State<'g> {
    /// `print2buff(strng, d)`.
    // R: deparse.c:print2buff
    fn print(&mut self, s: &[u8]) {
        if self.startline {
            self.startline = false;
            // if at the start of a line tab over
            self.printtab(self.indent);
        }
        self.buffer.extend_from_slice(s);
        self.len += s.len();
    }

    /// `printtab2buff(ntab, d)`: four spaces for each of the first four
    /// levels, then two.
    // R: deparse.c:printtab2buff
    fn printtab(&mut self, ntab: i32) {
        for i in 1..=ntab {
            let t: &[u8] = if i <= 4 { b"    " } else { b"  " };
            self.buffer.extend_from_slice(t);
            self.len += t.len();
        }
    }

    /// `writeline(d)`.
    // R: deparse.c:writeline
    fn writeline(&mut self) {
        if self.linenumber < self.maxlines {
            self.lines.push(std::mem::take(&mut self.buffer));
        }
        self.linenumber += 1;
        if self.linenumber >= self.maxlines {
            self.active = false;
        }
        // reset
        self.len = 0;
        self.buffer.clear();
        self.startline = true;
    }

    /// `linebreak(&lbreak, d)`.
    // R: deparse.c:linebreak
    fn linebreak(&mut self, lbreak: &mut bool) {
        if self.len > self.cutoff {
            if !*lbreak {
                *lbreak = true;
                self.indent += 1;
            }
            self.writeline();
        }
    }

    /// `quotify(name, quote)`: the name if it is syntactic (or empty), else
    /// `EncodeString(name, 0, quote, Rprt_adj_none)`.
    // R: deparse.c:quotify
    fn quotify(&mut self, name: Char<'_>, quote: u8) {
        let b = name.bytes();
        if b.is_empty() || is_valid_name(b) {
            self.print(b);
        } else {
            let mut e = Vec::new();
            encode_string(
                &mut e,
                Some(robigo_rtbuiltins::coerce::text(name)),
                0,
                quote,
                Justify::None,
                false,
                &self.pp,
            );
            self.print(&e);
        }
    }

    /// `translateChar(s)`.
    fn native(&self, c: Char<'_>) -> Result<Vec<u8>, Fail> {
        robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), self.cx.locale())
            .map(|b| b.into_owned())
            .map_err(|e| e.to_string())
    }

    /// `attr1(s, d)`: how the attributes of `s` are written; prints
    /// `structure(` when they need it.
    // R: deparse.c:attr1
    fn attr1(&mut self, s: Ref<'g>) -> Attr {
        let cx = self.cx;
        let nm = Names::get(cx, s);
        let nice_names = self.opts & NICE_NAMES != 0;
        let show_attr = self.opts & SHOWATTRIBUTES != 0;
        let has_names = nm.is_some();
        let mut attr = Attr::Unknown;
        if let Some(nm) = &nm {
            // ok only if there's no NA_character_,.. in names() nor all ""
            let ok_names = nice_names && usable_nice_names(cx, nm, robigo_rtbuiltins::util::is_vector_atomic(s));
            if !ok_names {
                attr = if show_attr { Attr::StrucNmsA } else { Attr::OkNames }; // even when not ok
            }
        }
        if attr == Attr::Unknown {
            for a in robigo_rtbuiltins::attrib::attrs(s) {
                let tag = a.tag.as_ref();
                if has_names && sym_is(tag, b"names") {
                    // also ok_names = true
                } else if show_attr && !sym_is(tag, b"srcref") {
                    attr = Attr::StrucAttr;
                    break;
                }
            }
        }
        if attr == Attr::Unknown {
            attr = if has_names { Attr::OkNames } else { Attr::Simple };
        }
        if attr >= Attr::StrucAttr {
            self.print(b"structure(");
        }
        attr
    }

    /// `attr2(s, d, not_names)`: `, tag = value` for each attribute, then
    /// `)`.
    // R: deparse.c:attr2
    fn attr2(&mut self, s: Ref<'g>, not_names: bool) -> DResult {
        for a in robigo_rtbuiltins::attrib::attrs(s) {
            let tag = a.tag.as_ref();
            if sym_is(tag, b"srcref") || (sym_is(tag, b"names") && not_names) {
                continue;
            }
            self.print(b", ");
            let n = a.tag.name().bytes();
            match n {
                b"dim" | b"dimnames" | b"names" | b"tsp" | b"levels" => self.print(n),
                _ => {
                    // TAG(a) might contain spaces etc
                    let opts = self.opts;
                    self.opts = SIMPLEDEPARSE; // turn off quote()ing
                    if is_valid_name(n) {
                        self.deparse2buff(tag)?;
                    } else {
                        self.print(b"\"");
                        self.deparse2buff(tag)?;
                        self.print(b"\"");
                    }
                    self.opts = opts;
                }
            }
            self.print(b" = ");
            let fnarg = self.fnarg;
            self.fnarg = true;
            self.deparse2buff(a.value)?;
            self.fnarg = fnarg;
        }
        self.print(b")");
        Ok(())
    }

    /// `deparse2buff(s, d)`: the recursive part of deparsing.
    // R: deparse.c:deparse2buff
    fn deparse2buff(&mut self, s: Ref<'g>) -> DResult {
        let cx = self.cx;
        let fnarg = self.fnarg;
        let d_opts_in = self.opts;
        self.fnarg = false;
        // This flag should only be set when recursing through the LHS of
        // binary ops, so by default we reset to zero
        let prev_left = self.left;
        self.left = 0;
        if !self.active {
            return Ok(());
        }
        if s.is_s4() {
            return Err("robigo: deparse of an S4 object is not implemented yet".into());
        }
        match s.sexptype() {
            NILSXP => self.print(b"NULL"),
            SYMSXP => {
                let name = sym_name(s);
                let doquote = d_opts_in & QUOTEEXPRESSIONS != 0 && !name.bytes().is_empty();
                let mut attr = Attr::Simple;
                if doquote {
                    attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(s) } else { Attr::Simple };
                    self.print(b"quote(");
                }
                if d_opts_in & S_COMPAT != 0 {
                    self.quotify(name, b'"');
                } else if self.backtick {
                    self.quotify(name, b'`');
                } else {
                    self.print(name.bytes());
                }
                if doquote {
                    self.print(b")");
                    if attr >= Attr::StrucAttr {
                        self.attr2(s, attr == Attr::StrucAttr)?;
                    }
                }
            }
            CHARSXP => {
                let t = self.native(s.as_char().unwrap())?;
                self.print(&t);
            }
            SPECIALSXP | BUILTINSXP => {
                self.print(b".Primitive(\"");
                self.print(prims::prim_name(cx, s).unwrap_or("").as_bytes());
                self.print(b"\")");
            }
            PROMSXP => {
                let p = match s.view() {
                    robigo_rtcore::View::Promise(p) => p,
                    _ => unreachable!(),
                };
                if self.opts & DELAYPROMISES != 0 {
                    self.sourceable = false;
                    self.print(b"<promise: ");
                    self.opts &= !QUOTEEXPRESSIONS; // don't want delay(quote())
                    self.deparse2buff(p.code())?;
                    self.opts = d_opts_in;
                    self.print(b">");
                } else if p.value().sexp() != cx.unbound().sexp() {
                    // eval uses env of promise: a forced one is its value
                    self.deparse2buff(p.value())?;
                } else {
                    return Err("robigo: deparse of an unforced promise (it forces it) is not implemented yet".into());
                }
            }
            CLOSXP => {
                let clo = match s.view() {
                    robigo_rtcore::View::Closure(c) => c,
                    _ => unreachable!(),
                };
                let attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(s) } else { Attr::Simple };
                if self.opts & USESOURCE != 0 && robigo_rtbuiltins::attrib::get_value(s, b"srcref").is_some() {
                    return Err("robigo: deparse of a function from its srcref is not implemented yet".into());
                }
                // We have established that we don't want to use the source
                // for this function
                self.opts &= SIMPLE_OPTS & !USESOURCE;
                self.print(b"function (");
                self.args2buff(clo.formals(), false, true)?;
                self.print(b") ");
                self.writeline();
                self.deparse2buff(clo.body())?;
                self.opts = d_opts_in;
                if attr >= Attr::StrucAttr {
                    self.attr2(s, attr == Attr::StrucAttr)?;
                }
            }
            ENVSXP => {
                self.sourceable = false;
                self.print(b"<environment>");
            }
            VECSXP => {
                let attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(s) } else { Attr::Simple };
                self.print(b"list(");
                self.opts = d_opts_in; // vec2buff() must use unchanged d
                self.vec2buff(s, attr == Attr::OkNames || attr == Attr::StrucAttr)?;
                self.opts |= NICE_NAMES;
                self.print(b")");
                if attr >= Attr::StrucAttr {
                    self.attr2(s, attr == Attr::StrucAttr)?;
                }
                self.opts = d_opts_in;
            }
            EXPRSXP => {
                let attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(s) } else { Attr::Simple };
                if s.len() == Some(0) {
                    self.print(b"expression()");
                } else {
                    let loc = self.opts;
                    self.print(b"expression(");
                    self.opts &= SIMPLE_OPTS;
                    self.vec2buff(s, attr == Attr::OkNames || attr == Attr::StrucAttr)?;
                    self.opts = loc;
                    self.print(b")");
                }
                if attr >= Attr::StrucAttr {
                    self.attr2(s, attr == Attr::StrucAttr)?;
                }
                self.opts = d_opts_in;
            }
            LISTSXP => {
                let attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(s) } else { Attr::Simple };
                // pairlist(x=) cannot be evaluated, hence with missings we
                // use as.pairlist(alist(...)) to allow evaluation of
                // deparsed formals
                let missing = cells(s).any(|c| c.car().sexp() == cx.missing_arg().sexp());
                self.print(if missing { b"as.pairlist(alist(" as &[u8] } else { b"pairlist(" });
                self.inlist += 1;
                let all: Vec<_> = cells(s).collect();
                for (k, c) in all.iter().enumerate() {
                    if c.tag().sexptype() != NILSXP {
                        self.opts = SIMPLEDEPARSE; // turn off quote()ing
                        self.deparse2buff(c.tag())?;
                        self.opts = d_opts_in;
                        self.print(b" = ");
                    }
                    self.deparse2buff(c.car())?;
                    if k + 1 < all.len() {
                        self.print(b", ");
                    }
                }
                self.print(if missing { b"))" as &[u8] } else { b")" });
                self.inlist -= 1;
                if attr >= Attr::StrucAttr {
                    self.attr2(s, attr == Attr::StrucAttr)?;
                }
            }
            LANGSXP => self.language(s, fnarg, prev_left, d_opts_in)?,
            STRSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | RAWSXP => self.vector2buff(s)?,
            EXTPTRSXP => {
                self.sourceable = false;
                let addr = match s.view() {
                    robigo_rtcore::View::ExtPtr(p) => p.addr(),
                    _ => unreachable!(),
                };
                let t = format!("<pointer: {}>", crate::print::c_pointer(addr as usize));
                let t = &t.as_bytes()[..t.len().min(31)];
                self.print(t);
            }
            BCODESXP => {
                self.sourceable = false;
                self.print(b"<bytecode>");
            }
            WEAKREFSXP => {
                self.sourceable = false;
                self.print(b"<weak reference>");
            }
            OBJSXP => {
                self.sourceable = false;
                self.print(b"<object>");
            }
            t => {
                // UNIMPLEMENTED_TYPE("deparse2buff", s)
                let msg =
                    format!("unimplemented type '{}' in '{}'\n", robigo_rtbuiltins::util::type2char(t), "deparse2buff");
                return Err(msg);
            }
        }
        self.left = prev_left;
        Ok(())
    }

    /// The LANGSXP case of `deparse2buff`.
    // R: deparse.c:deparse2buff (case LANGSXP)
    fn language(&mut self, s0: Ref<'g>, fnarg: bool, prev_left: u8, d_opts_in: i32) -> DResult {
        let cx = self.cx;
        if robigo_rtbuiltins::attrib::has_attrib(s0) {
            self.sourceable = false;
        }
        let op = s0.as_cons().unwrap().car();
        let mut doquote = false;
        let maybe_quote = d_opts_in & QUOTEEXPRESSIONS != 0;
        if maybe_quote {
            // do *not* quote() formulas
            doquote = !sym_is(op, b"~");
            if doquote {
                self.print(b"quote(");
                self.opts &= SIMPLE_OPTS;
            } else {
                self.opts &= !QUOTEEXPRESSIONS;
            }
        }
        if op.sexptype() == SYMSXP {
            let prim = sym_prim(cx, op);
            let userbinop = prim.is_none() && is_user_binop(op);
            if prim.is_some() || userbinop {
                let s = s0.as_cons().unwrap().cdr();
                let opname = sym_name(op).bytes();
                let mut fop = if userbinop {
                    if Names::get(cx, s).is_none() {
                        // not quite right for spacing, but can't be unary
                        PpInfo { kind: PpKind::Binary2, prec: Prec::Percent, right_assoc: false }
                    } else {
                        // if args are named, deparse as function call (PR#15350)
                        PpInfo { kind: PpKind::Funcall, prec: Prec::Fn, right_assoc: false }
                    }
                } else {
                    prim.unwrap().0
                };
                let primname = prim.map(|p| p.1).unwrap_or("");
                match fop.kind {
                    PpKind::Binary => match plen(s) {
                        1 => {
                            fop.kind = PpKind::Unary;
                            if fop.prec == Prec::Sum {
                                // binary +/- precedence upgraded as unary
                                fop.prec = Prec::Sign;
                            }
                        }
                        2 => {}
                        _ => fop.kind = PpKind::Funcall,
                    },
                    PpKind::Binary2 => {
                        if plen(s) != 2 {
                            fop.kind = PpKind::Funcall;
                        } else if userbinop {
                            fop.kind = PpKind::Binary;
                        }
                    }
                    PpKind::Dollar => {
                        if plen(s) != 2 {
                            fop.kind = PpKind::Funcall;
                        } else {
                            let rhs = nth(cx, s, 1);
                            let valid_string = rhs.sexptype() == STRSXP
                                && rhs.len().unwrap_or(0) > 0
                                && rhs.as_vector::<Char<'_>>().unwrap().get(0) != cx.na_string();
                            if rhs.sexptype() != SYMSXP && !valid_string {
                                fop.kind = PpKind::Funcall;
                            }
                        }
                    }
                    _ => {}
                }
                let car = nth(cx, s, 0);
                let cadr = nth(cx, s, 1);
                match fop.kind {
                    PpKind::If => {
                        self.print(b"if (");
                        // print the predicate
                        self.deparse2buff(car)?;
                        self.print(b") ");
                        let mut lookahead = false;
                        if self.incurly != 0 && self.inlist == 0 {
                            lookahead = curlyahead(cadr);
                            if !lookahead {
                                self.writeline();
                                self.indent += 1;
                            }
                        }
                        // need to find out if there is an else
                        if plen(s) > 2 {
                            self.deparse2buff(cadr)?;
                            if self.incurly != 0 && self.inlist == 0 {
                                self.writeline();
                                if !lookahead {
                                    self.indent -= 1;
                                }
                            } else {
                                self.print(b" ");
                            }
                            self.print(b"else ");
                            self.deparse2buff(nth(cx, s, 2))?;
                        } else {
                            self.deparse2buff(cadr)?;
                            if self.incurly != 0 && !lookahead && self.inlist == 0 {
                                self.indent -= 1;
                            }
                        }
                    }
                    PpKind::While => {
                        self.print(b"while (");
                        self.deparse2buff(car)?;
                        self.print(b") ");
                        self.deparse2buff(cadr)?;
                    }
                    PpKind::For => {
                        self.print(b"for (");
                        self.deparse2buff(car)?;
                        self.print(b" in ");
                        self.deparse2buff(cadr)?;
                        self.print(b") ");
                        self.deparse2buff(nth(cx, s, 2))?;
                    }
                    PpKind::Repeat => {
                        self.print(b"repeat ");
                        self.deparse2buff(car)?;
                    }
                    PpKind::Curly => {
                        self.print(b"{");
                        self.incurly += 1;
                        self.indent += 1;
                        self.writeline();
                        for c in cells(s) {
                            self.deparse2buff(c.car())?;
                            self.writeline();
                        }
                        self.indent -= 1;
                        self.print(b"}");
                        self.incurly -= 1;
                    }
                    PpKind::Paren => {
                        self.print(b"(");
                        self.deparse2buff(car)?;
                        self.print(b")");
                    }
                    PpKind::Subset => {
                        let parens = needsparens(cx, fop, car, true, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        // PRIMVAL(SYMVALUE(op)) == 1 is `[`
                        let single = primname == "[";
                        self.print(if single { b"[" as &[u8] } else { b"[[" });
                        self.args2buff(s.as_cons().map(|c| c.cdr()).unwrap_or(cx.nil()), false, false)?;
                        self.print(if single { b"]" as &[u8] } else { b"]]" });
                    }
                    PpKind::Funcall | PpKind::Return => {
                        let name = sym_name(op);
                        if self.backtick {
                            self.quotify(name, b'`');
                        } else {
                            self.quotify(name, b'"');
                        }
                        self.print(b"(");
                        self.inlist += 1;
                        self.args2buff(s, false, false)?;
                        self.inlist -= 1;
                        self.print(b")");
                    }
                    PpKind::Foreign => {
                        self.print(opname); // ASCII
                        self.print(b"(");
                        self.inlist += 1;
                        self.args2buff(s, true, false)?;
                        self.inlist -= 1;
                        self.print(b")");
                    }
                    PpKind::Function => {
                        let srcs = nth(cx, s, 2);
                        if self.opts & USESOURCE == 0 || srcs.sexptype() != STRSXP {
                            self.print(opname); // ASCII
                            self.print(b"(");
                            self.args2buff(car, false, true)?;
                            self.print(b") ");
                            self.deparse2buff(cadr)?;
                        } else {
                            let v = srcs.as_vector::<Char<'_>>().unwrap();
                            for i in 0..v.len() {
                                let t = self.native(v.get(i))?;
                                self.print(&t);
                                self.writeline();
                            }
                        }
                    }
                    PpKind::Assign | PpKind::Assign2 => {
                        let outerparens = fnarg && opname == b"=";
                        if outerparens {
                            self.print(b"(");
                        }
                        let mut parens = needsparens(cx, fop, car, true, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { fop.prec as u8 };
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        self.print(b" ");
                        self.print(opname); // ASCII
                        self.print(b" ");
                        parens = needsparens(cx, fop, cadr, false, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { prev_left };
                        self.deparse2buff(cadr)?;
                        if parens {
                            self.print(b")");
                        }
                        if outerparens {
                            self.print(b")");
                        }
                        self.left = 0;
                    }
                    PpKind::Dollar => {
                        let mut parens = needsparens(cx, fop, car, true, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { fop.prec as u8 };
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        self.print(opname); // ASCII
                        // temp fix to handle printing of x$a's
                        let str0 = (cadr.sexptype() == STRSXP)
                            .then(|| cadr.as_vector::<Char<'_>>().unwrap())
                            .filter(|v| !v.is_empty())
                            .map(|v| v.get(0));
                        if let Some(c) = str0
                            && is_valid_name(c.bytes())
                        {
                            self.deparse2buff(c.as_ref())?;
                        } else {
                            parens = needsparens(cx, fop, cadr, false, prev_left);
                            if parens {
                                self.print(b"(");
                            }
                            self.left = if parens { 0 } else { prev_left };
                            self.deparse2buff(cadr)?;
                            if parens {
                                self.print(b")");
                            }
                        }
                        self.left = 0;
                    }
                    PpKind::Binary => {
                        let mut lbreak = false;
                        let mut parens = needsparens(cx, fop, car, true, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { fop.prec as u8 };
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        self.print(b" ");
                        self.print(opname); // ASCII
                        self.print(b" ");
                        self.linebreak(&mut lbreak);
                        parens = needsparens(cx, fop, cadr, false, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { prev_left };
                        self.deparse2buff(cadr)?;
                        if parens {
                            self.print(b")");
                        }
                        if lbreak {
                            self.indent -= 1;
                        }
                        self.left = 0;
                    }
                    PpKind::Binary2 => {
                        // no space between op and args
                        let mut parens = needsparens(cx, fop, car, true, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { fop.prec as u8 };
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        self.print(opname); // ASCII
                        parens = needsparens(cx, fop, cadr, false, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { prev_left };
                        self.deparse2buff(cadr)?;
                        if parens {
                            self.print(b")");
                        }
                        self.left = 0;
                    }
                    PpKind::Unary => {
                        self.print(opname); // ASCII
                        let parens = needsparens(cx, fop, car, false, prev_left);
                        if parens {
                            self.print(b"(");
                        }
                        self.left = if parens { 0 } else { prev_left };
                        self.deparse2buff(car)?;
                        if parens {
                            self.print(b")");
                        }
                        self.left = 0;
                    }
                    PpKind::Break => self.print(b"break"),
                    PpKind::Next => self.print(b"next"),
                    PpKind::Subass => {
                        if self.opts & S_COMPAT != 0 {
                            self.print(b"\"");
                            self.print(opname); // ASCII
                            self.print(b"'(");
                        } else {
                            self.print(b"`");
                            self.print(opname); // ASCII
                            self.print(b"`(");
                        }
                        self.args2buff(s, false, false)?;
                        self.print(b")");
                    }
                    PpKind::Invalid => {
                        self.sourceable = false;
                        return Err("unimplemented feature in deparse2buff".into());
                    }
                }
            } else {
                // a closure, or unbound in base: a regular call (`::` and
                // `:::`, the special cases here, are primitives in 4.6.2)
                if d_opts_in & S_COMPAT != 0 {
                    self.quotify(sym_name(op), b'\'');
                } else {
                    self.quotify(sym_name(op), b'`');
                }
                self.print(b"(");
                self.args2buff(s0.as_cons().unwrap().cdr(), false, false)?;
                self.print(b")");
            }
        } else {
            // a function object, or a lambda expression
            if parenthesize_caller(cx, op) {
                self.print(b"(");
                self.deparse2buff(op)?;
                self.print(b")");
            } else {
                self.deparse2buff(op)?;
            }
            self.print(b"(");
            self.args2buff(s0.as_cons().unwrap().cdr(), false, false)?;
            self.print(b")");
        }
        if maybe_quote {
            self.opts = d_opts_in;
            if doquote {
                self.print(b")");
            }
        }
        Ok(())
    }

    /// `deparse2buf_name(nv, i, d)`: `name = ` for element `i`, when it
    /// has a name.
    // R: deparse.c:deparse2buf_name
    fn name_eq(&mut self, nv: &Option<Names<'g>>, i: usize) -> DResult {
        // (NA_STRING is a name: CHAR(NA_STRING) is "NA", not syntactic)
        let Some(c) = nv.as_ref().and_then(|nv| nv.at(i)) else { return Ok(()) };
        if c.bytes().is_empty() {
            return Ok(());
        }
        let t = self.native(c)?;
        if is_valid_name(&t) {
            self.deparse2buff(c.as_ref())?;
        } else if self.backtick {
            self.print(b"`");
            self.deparse2buff(c.as_ref())?;
            self.print(b"`");
        } else {
            self.print(b"\"");
            self.deparse2buff(c.as_ref())?;
            self.print(b"\"");
        }
        self.print(b" = ");
        Ok(())
    }

    /// One element of an atomic vector as `EncodeElement(x, i, quote, '.')`
    /// writes it: in its own format, at 15 digits.
    // R: printutils.c:EncodeElement0
    fn encode_element(&self, x: Ref<'_>, i: usize, quote: u8, out: &mut Vec<u8>) {
        encode_element(self.cx, x, i, quote, &self.pp, out);
    }

    /// `vector2buff(vector, d)`: an atomic vector.
    // R: deparse.c:vector2buff
    fn vector2buff(&mut self, vector: Ref<'g>) -> DResult {
        let cx = self.cx;
        let d_opts_in = self.opts;
        let t = vector.sexptype();
        let tlen = vector.len().unwrap();
        let quote = if t == STRSXP { b'"' } else { 0 };
        let mut surround = false;
        // := true iff integer sequence 'm:n' (up *or* down)
        let mut int_seq = false;
        let ints = (t == INTSXP).then(|| vector.as_vector::<i32>().unwrap());
        if let Some(v) = ints
            && tlen > 1
        {
            let d_i = v.get(1) as f64 - v.get(0) as f64;
            int_seq = v.get(0) != i32::MIN && v.get(1) != i32::MIN && d_i.abs() == 1.0;
            if int_seq {
                for i in 2..tlen {
                    if v.get(i) == i32::MIN || (v.get(i) as f64 - v.get(i - 1) as f64) != d_i {
                        int_seq = false;
                        break;
                    }
                }
            }
        }
        let mut nv = None;
        let mut do_names = d_opts_in & SHOW_ATTR_OR_NMS != 0;
        if do_names {
            nv = Names::get(cx, vector);
            if nv.is_none() {
                do_names = false;
            }
        }
        let str_names = do_names && (int_seq || tlen == 0);
        let mut need_c = tlen > 1;
        if str_names {
            self.opts &= !NICE_NAMES;
        }
        let attr = if d_opts_in & SHOW_ATTR_OR_NMS != 0 { self.attr1(vector) } else { Attr::Simple };
        if do_names {
            do_names = attr == Attr::OkNames || attr == Attr::StrucAttr;
        }
        if !need_c {
            need_c = do_names; // c(a = *) but not c(1)
        }
        let mut buf = Vec::new();
        if tlen == 0 {
            self.print(match t {
                LGLSXP => b"logical(0)" as &[u8],
                INTSXP => b"integer(0)",
                REALSXP => b"numeric(0)",
                CPLXSXP => b"complex(0)",
                STRSXP => b"character(0)",
                _ => b"raw(0)",
            });
        } else if let Some(vec) = ints {
            // We treat integer separately, as S_compatible is relevant.
            if int_seq {
                // m:n
                self.encode_element(vector, 0, b'"', &mut buf);
                self.print(&buf);
                self.print(b":");
                buf.clear();
                self.encode_element(vector, tlen - 1, b'"', &mut buf);
                self.print(&buf);
            } else {
                let add_l = self.opts & KEEPINTEGER != 0 && self.opts & S_COMPAT == 0;
                let mut all_na = self.opts & KEEPNA != 0 || add_l;
                if (0..tlen).any(|i| vec.get(i) != i32::MIN) {
                    all_na = false;
                }
                if self.opts & KEEPINTEGER != 0 && self.opts & S_COMPAT != 0 {
                    self.print(b"as.integer(");
                    surround = true;
                }
                all_na = all_na && self.opts & S_COMPAT == 0;
                if need_c {
                    self.print(b"c(");
                }
                for i in 0..tlen {
                    if do_names {
                        self.name_eq(&nv, i)?;
                    }
                    if all_na && vec.get(i) == i32::MIN {
                        self.print(b"NA_integer_");
                    } else {
                        buf.clear();
                        self.encode_element(vector, i, quote, &mut buf);
                        self.print(&buf);
                        if add_l && vec.get(i) != i32::MIN {
                            self.print(b"L");
                        }
                    }
                    if i < tlen - 1 {
                        self.print(b", ");
                    }
                    if tlen > 1 && self.len > self.cutoff {
                        self.writeline();
                    }
                    if !self.active {
                        break;
                    }
                }
                if need_c {
                    self.print(b")");
                }
                if surround {
                    self.print(b")");
                }
            }
        } else {
            let keepna = self.opts & KEEPNA != 0;
            let scompat = self.opts & S_COMPAT != 0;
            let mut all_na = keepna;
            let reals = (t == REALSXP).then(|| vector.as_vector::<f64>().unwrap());
            let cplx = (t == CPLXSXP).then(|| vector.as_vector::<Rcomplex>().unwrap());
            let strs = (t == STRSXP).then(|| vector.as_vector::<Char<'_>>().unwrap());
            if keepna && let Some(v) = reals {
                if (0..tlen).any(|i| !is_na(v.get(i))) {
                    all_na = false;
                }
                if all_na && scompat {
                    self.print(b"as.double(");
                    surround = true;
                }
            } else if keepna && let Some(v) = cplx {
                if (0..tlen).any(|i| !is_na(v.get(i).r) && !is_na(v.get(i).i)) {
                    all_na = false;
                }
                if all_na && scompat {
                    self.print(b"as.complex(");
                    surround = true;
                }
            } else if keepna && let Some(v) = strs {
                if (0..tlen).any(|i| v.get(i) != cx.na_string()) {
                    all_na = false;
                }
                if all_na && scompat {
                    self.print(b"as.character(");
                    surround = true;
                }
            } else if t == RAWSXP {
                self.print(b"as.raw(");
                surround = true;
            }
            if need_c {
                self.print(b"c(");
            }
            all_na = all_na && !scompat;
            for i in 0..tlen {
                if do_names {
                    self.name_eq(&nv, i)?;
                }
                buf.clear();
                if all_na && reals.is_some_and(|v| is_na(v.get(i))) {
                    buf.extend_from_slice(b"NA_real_");
                } else if let Some(z) = cplx.map(|v| v.get(i))
                    && is_na(z.r)
                    && is_na(z.i)
                {
                    if all_na {
                        buf.extend_from_slice(b"NA_complex_");
                    } else {
                        self.encode_element(vector, i, quote, &mut buf);
                    }
                } else if let Some(z) = cplx.map(|v| v.get(i))
                    && (z.r.is_nan() || !z.i.is_finite())
                {
                    self.encode_non_finite_complex(z, &mut buf);
                } else if all_na && strs.is_some_and(|v| v.get(i) == cx.na_string()) {
                    buf.extend_from_slice(b"NA_character_");
                } else if t == REALSXP && scompat {
                    return Err("robigo: deparse with control \"S_compatible\" is not implemented yet".into());
                } else if t == RAWSXP {
                    encode_raw(&mut buf, vector.as_vector::<u8>().unwrap().get(i), b"0x");
                } else if matches!(t, REALSXP | CPLXSXP) && self.opts & (HEXNUMERIC | DIGITS17) != 0 {
                    let finite = match t {
                        REALSXP => reals.unwrap().get(i).is_finite(),
                        _ => {
                            let z = cplx.unwrap().get(i);
                            z.r.is_finite() && z.i.is_finite()
                        }
                    };
                    if finite {
                        return Err(
                            "robigo: deparse with control \"hexNumeric\" or \"digits17\" is not implemented yet".into(),
                        );
                    }
                    self.encode_element(vector, i, quote, &mut buf);
                } else {
                    self.encode_element(vector, i, quote, &mut buf);
                }
                self.print(&buf);
                if i < tlen - 1 {
                    self.print(b", ");
                }
                if tlen > 1 && self.len > self.cutoff {
                    self.writeline();
                }
                if !self.active {
                    break;
                }
            }
            if need_c {
                self.print(b")");
            }
            if surround {
                self.print(b")");
            }
        }
        if attr >= Attr::StrucAttr {
            self.attr2(vector, attr == Attr::StrucAttr)?;
        }
        if str_names {
            self.opts = d_opts_in;
        }
        Ok(())
    }

    /// `EncodeNonFiniteComplexElement(x, buff)`: `complex(real=, imaginary=)`,
    /// since `1+Infi` or `3+NaNi` do not parse.
    // R: deparse.c:EncodeNonFiniteComplexElement
    fn encode_non_finite_complex(&self, x: Rcomplex, out: &mut Vec<u8>) {
        let (fr, fi) = format_complex(&[(x.r, x.i)], 0, &self.pp);
        out.extend_from_slice(b"complex(real=");
        encode_real0(out, x.r, fr, b".", &self.pp);
        out.extend_from_slice(b", imaginary=");
        encode_real0(out, x.i, fi, b".", &self.pp);
        out.push(b')');
    }

    /// `vec2buff(v, d, do_names)`: the elements of a list or expression
    /// vector.
    // R: deparse.c:vec2buff
    fn vec2buff(&mut self, v: Ref<'g>, do_names: bool) -> DResult {
        let cx = self.cx;
        let mut lbreak = false;
        let n = v.len().unwrap();
        let nv = if do_names { Names::get(cx, v) } else { None };
        if self.opts & USESOURCE != 0
            && robigo_rtbuiltins::attrib::get_value(v, b"srcref").is_some_and(|s| s.sexptype() == VECSXP)
        {
            return Err("robigo: deparse from srcrefs is not implemented yet".into());
        }
        let elts = match v.view() {
            robigo_rtcore::View::List(l) | robigo_rtcore::View::Expr(l) => l,
            _ => unreachable!(),
        };
        for i in 0..n {
            if i > 0 {
                self.print(b", ");
            }
            self.linebreak(&mut lbreak);
            if nv.is_some() {
                self.name_eq(&nv, i)?;
            }
            self.deparse2buff(elts.get(i))?;
        }
        if lbreak {
            self.indent -= 1;
        }
        Ok(())
    }

    /// `args2buff(arglist, lineb, formals, d)`: the arguments of a call, or
    /// a function's formals.
    // R: deparse.c:args2buff
    fn args2buff(&mut self, arglist: Ref<'g>, _lineb: bool, formals: bool) -> DResult {
        let cx = self.cx;
        let mut lbreak = false;
        let mut a = arglist;
        while a.sexptype() != NILSXP {
            if !matches!(a.sexptype(), LISTSXP | LANGSXP) {
                return Err("badly formed function expression".into());
            }
            let c = a.as_cons().unwrap();
            let tag = c.tag();
            if tag.sexptype() != NILSXP {
                let name = sym_name(tag);
                if name.bytes() == b"..." {
                    self.print(b"...");
                } else if self.backtick {
                    self.quotify(name, b'`');
                } else {
                    self.quotify(name, b'"');
                }
                if formals {
                    if c.car().sexp() != cx.missing_arg().sexp() {
                        self.print(b" = ");
                        self.fnarg = true;
                        self.deparse2buff(c.car())?;
                    }
                } else {
                    self.print(b" = ");
                    if c.car().sexp() != cx.missing_arg().sexp() {
                        self.fnarg = true;
                        self.deparse2buff(c.car())?;
                    }
                }
            } else {
                self.fnarg = true;
                self.deparse2buff(c.car())?;
            }
            a = c.cdr();
            if a.sexptype() != NILSXP {
                self.print(b", ");
                self.linebreak(&mut lbreak);
            }
        }
        if lbreak {
            self.indent -= 1;
        }
        Ok(())
    }
}

/// `usable_nice_names(x, isAtomic)`: names with no `NA_character_` and not
/// all empty (for an atomic vector, none "recursive" or "use.names").
// R: deparse.c:usable_nice_names
fn usable_nice_names<'g>(cx: &'g Cx, nm: &Names<'g>, is_atomic: bool) -> bool {
    let Some(v) = nm.strings(cx) else { return true };
    let mut all_0 = true;
    for c in v {
        if c == cx.na_string() || (is_atomic && matches!(c.bytes(), b"recursive" | b"use.names")) {
            return false;
        } else if all_0 && !c.bytes().is_empty() {
            all_0 = false;
        }
    }
    !all_0
}

/// `EncodeElement0(x, indx, quote, dec)`: element `i` of an atomic vector
/// in its own format (`formatX` of the one element, then `EncodeX`), under
/// the print settings `pp`.
// R: printutils.c:EncodeElement0
pub fn encode_element(cx: &Cx, x: Ref<'_>, i: usize, quote: u8, pp: &PrintParams, out: &mut Vec<u8>) {
    match x.view() {
        robigo_rtcore::View::Lgl(v) => {
            let e = [v.get(i).0];
            let w = format_logical(&e, pp);
            encode_logical(out, e[0], w, pp);
        }
        robigo_rtcore::View::Int(v) => {
            let e = [v.get(i)];
            let w = format_integer(&e, pp);
            encode_integer(out, e[0], w, pp);
        }
        robigo_rtcore::View::Real(v) => {
            let e = [v.get(i)];
            let f = format_real(&e, 0, pp);
            encode_real0(out, e[0], f, b".", pp);
        }
        robigo_rtcore::View::Str(v) => {
            let c = v.get(i);
            let s: Option<Str<'_>> = (c != cx.na_string()).then(|| robigo_rtbuiltins::coerce::text(c));
            let w = format_string([s], quote, pp);
            encode_string(out, s, w, quote, Justify::Left, false, pp);
        }
        robigo_rtcore::View::Cplx(v) => {
            let z = v.get(i);
            let (fr, fi) = format_complex(&[(z.r, z.i)], 0, pp);
            encode_complex(out, (z.r, z.i), fr, fi, b".", pp);
        }
        robigo_rtcore::View::Raw(v) => encode_raw(out, v.get(i), b""),
        _ => panic!("robigo: EncodeElement of a non-atomic vector"),
    }
}
