//! Regular expressions: `strsplit`, `grep`, `grepl`, `sub`, `gsub`,
//! `regexpr`, `gregexpr` and `regexec` (GNU R's grep.c).
//!
//! GNU R matches extended regular expressions with TRE and `perl = TRUE`
//! patterns with PCRE2. robigo translates both dialects to the syntax of the
//! Rust `regex` family and matches with:
//!
//! * TRE (default): a port of TRE's parser (tre-parse.c) that emits an
//!   equivalent `regex-automata` pattern. TRE uses POSIX leftmost-longest
//!   semantics: the leftmost match start is found with a leftmost-first
//!   engine and the longest end with a DFA in `MatchKind::All` mode. Like
//!   TRE in R, every search runs on the remaining substring as a fresh
//!   string (with `REG_NOTBOL` where R sets it). Patterns with
//!   back-references are matched by `fancy-regex` (leftmost-first).
//! * PCRE (`perl = TRUE`): translated to `fancy-regex`, which provides
//!   look-around, back-references, atomic groups and `\K`. As R does not
//!   enable PCRE2_UCP, `\w`, `\d`, `\s` and `\b` are ASCII-only.
//!
//! Known differences from GNU R:
//! * TRE: submatch positions follow leftmost-first rather than POSIX rules
//!   for ambiguous patterns; `\B` is not suppressed at the start/end of the
//!   string as TRE does; with Unicode text, the longest-match DFA may give
//!   up on `\b`/`\<`/`\>`, falling back to leftmost-first; TRE's
//!   approximate-matching `{...}` syntax is not supported.
//! * PCRE: recursion, callouts, `(*VERB)`s other than `(*UCP)`/`(*UTF)` and
//!   some error messages are not supported; compile errors other than the
//!   common ones report the underlying engine's message.
//! * `useBytes = TRUE` with non-ASCII input matches characters (robigo strings
//!   are always valid UTF-8) but reports byte positions.

use std::cell::RefCell;
use std::rc::Rc;

use regex_automata::hybrid::dfa::{Cache as DfaCache, DFA};
use regex_automata::meta::Regex as MetaRegex;
use regex_automata::{Anchored, Input, MatchKind};

use super::character::{as_bool2, check_arity_internal, copy_all_attrs, simple_lower, simple_upper};
use super::internal;
use crate::coerce::as_logical;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("strsplit", do_strsplit, 5),
    internal("grep", do_grep, 8),
    internal("grepl", do_grepl, 8),
    internal("sub", do_sub, 7),
    internal("gsub", do_gsub, 7),
    internal("regexpr", do_regexpr, 6),
    internal("gregexpr", do_gregexpr, 6),
    internal("regexec", do_regexec, 5),
];

// ======================================================================
// TRE syntax -> regex-automata syntax

const REG_BADPAT: &str = "Invalid regexp";
const REG_ECOLLATE: &str = "Unknown collating element";
const REG_ECTYPE: &str = "Unknown character class name";
const REG_EESCAPE: &str = "Trailing backslash";
const REG_ESUBREG: &str = "Invalid back reference";
const REG_EBRACK: &str = "Missing ']'";
const REG_EPAREN: &str = "Missing ')'";
const REG_EBRACE: &str = "Missing '}'";
const REG_BADBR: &str = "Invalid contents of {}";
const REG_ERANGE: &str = "Invalid character range";
const REG_BADRPT: &str = "Invalid use of repetition operators";
const RE_DUP_MAX: i64 = 255;

/// TRE's word characters (`IS_WORD_CHAR`: alnum or `_`) and their complement.
const TRE_WORD: &str = r"[\p{Alphabetic}\p{Nd}_]";
const TRE_NONWORD: &str = r"[^\p{Alphabetic}\p{Nd}_]";

/// A character class that can never match (used for `^` under REG_NOTBOL).
const NEVER: &str = r"[^\x{0}-\x{10FFFF}]";

/// Escape one character for use in a pattern or inside a class.
fn esc(c: char, out: &mut String) {
    if c.is_ascii_alphanumeric() || c == '_' {
        out.push(c);
    } else {
        out.push_str(&format!("\\x{{{:X}}}", c as u32));
    }
}

fn esc_str(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        esc(c, &mut out);
    }
    out
}

// glibc's character classes in a UTF-8 locale, as class items.
const CLS_SPACE: &str = r"\t\n\x{B}\x{C}\r\x{20}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{2028}\x{2029}\x{205F}\x{3000}";
const CLS_BLANK: &str = r"\t\x{20}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{205F}\x{3000}";
const CLS_CNTRL: &str = r"\x{0}-\x{1F}\x{7F}-\x{9F}\x{2028}\x{2029}";
const CLS_PRINT: &str = r"[^\x{0}-\x{1F}\x{7F}-\x{9F}\x{2028}\x{2029}\p{Cn}]";
const CLS_GRAPH: &str = r"[^\x{0}-\x{20}\x{7F}-\x{9F}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{2028}\x{2029}\x{205F}\x{3000}\p{Cn}]";
const CLS_PUNCT: &str = r"[^\x{0}-\x{20}\x{7F}-\x{9F}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{2028}\x{2029}\x{205F}\x{3000}\p{Cn}\p{Alphabetic}\p{Nd}]";

/// Class items for a POSIX class name (TRE's `tre_ctype`, via glibc's
/// `wctype` in a UTF-8 locale).
fn tre_ctype(name: &str) -> Option<&'static str> {
    Some(match name {
        "alnum" => r"\p{Alphabetic}\p{Nd}",
        "alpha" => r"\p{Alphabetic}[\p{Nd}--0-9]",
        "blank" => CLS_BLANK,
        "cntrl" => CLS_CNTRL,
        "digit" => "0-9",
        "graph" => CLS_GRAPH,
        "lower" => r"\p{Lowercase}",
        "print" => CLS_PRINT,
        "punct" => CLS_PUNCT,
        "space" => CLS_SPACE,
        "upper" => r"\p{Uppercase}",
        "xdigit" => "0-9A-Fa-f",
        _ => return None,
    })
}

struct TreParser {
    re: Vec<char>,
    i: usize,
    depth: i32,
    icase: bool,
    newline: bool,
    ungreedy: bool,
    literal: bool,
    notbol: bool,
    nsub: usize,
    max_backref: usize,
    has_bol: bool,
    has_minimal: bool,
    has_lookaround: bool,
}

pub struct TreOut {
    pub pattern: String,
    pub nsub: usize,
    pub has_backref: bool,
    pub has_bol: bool,
    pub has_minimal: bool,
    /// Needs look-around (TRE's `\B`): matched by fancy-regex.
    pub has_lookaround: bool,
}

type TreRes<T> = Result<T, &'static str>;

impl TreParser {
    fn peek(&self) -> Option<char> {
        self.re.get(self.i).copied()
    }

    fn at(&self, k: usize) -> Option<char> {
        self.re.get(self.i + k).copied()
    }

    fn lit(&self, c: char) -> String {
        let mut s = String::new();
        if self.icase && (c.is_lowercase() || c.is_uppercase()) {
            s.push_str("(?i:");
            esc(c, &mut s);
            s.push(')');
        } else {
            esc(c, &mut s);
        }
        s
    }

    // PARSE_RE + PARSE_UNION
    fn parse_re(&mut self) -> TreRes<String> {
        let mut alts = vec![self.parse_branch()?];
        loop {
            if self.literal {
                break;
            }
            match self.peek() {
                Some('|') => {
                    self.i += 1;
                    alts.push(self.parse_branch()?);
                }
                Some(')') => {
                    self.i += 1;
                    break;
                }
                _ => break,
            }
        }
        Ok(alts.join("|"))
    }

    // PARSE_BRANCH + PARSE_CATENATION
    fn parse_branch(&mut self) -> TreRes<String> {
        let mut s = self.parse_piece()?;
        loop {
            let Some(c) = self.peek() else { break };
            if !self.literal {
                if c == '|' {
                    break;
                }
                if c == ')' && self.depth > 0 {
                    self.depth -= 1;
                    break;
                }
            }
            s.push_str(&self.parse_piece()?);
        }
        Ok(s)
    }

    // PARSE_PIECE + PARSE_POSTFIX
    fn parse_piece(&mut self) -> TreRes<String> {
        let mut atom = self.parse_atom()?;
        loop {
            if self.literal {
                break;
            }
            let Some(c) = self.peek() else { break };
            match c {
                '*' | '+' | '?' => {
                    let mut minimal = self.ungreedy;
                    if let Some(n) = self.at(1) {
                        if n == '?' {
                            minimal = !self.ungreedy;
                            self.i += 1;
                        } else if n == '*' || n == '+' {
                            return Err(REG_BADRPT);
                        }
                    }
                    self.i += 1;
                    let op = match c {
                        '*' => "*",
                        '+' => "+",
                        _ => "?",
                    };
                    self.has_minimal |= minimal;
                    atom = format!("(?:{}){}{}", atom, op, if minimal { "?" } else { "" });
                }
                '{' => {
                    self.i += 1;
                    atom = self.parse_bound(atom)?;
                }
                _ => break,
            }
        }
        Ok(atom)
    }

    fn parse_int(&mut self) -> i64 {
        let mut num: i64 = -1;
        let mut overflow = false;
        while let Some(c) = self.peek() {
            let Some(d) = c.to_digit(10) else { break };
            if num < 0 {
                num = 0;
            }
            if num <= (i32::MAX as i64 - 9) / 10 {
                num = num * 10 + d as i64;
            } else {
                overflow = true;
            }
            self.i += 1;
        }
        if overflow { -1 } else { num }
    }

    // R: tre-parse.c:tre_parse_bound (without approximate matching)
    fn parse_bound(&mut self, atom: String) -> TreRes<String> {
        let start = self.i;
        let mut min = -1;
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            min = self.parse_int();
        }
        let mut max = min;
        if self.peek() == Some(',') {
            self.i += 1;
            max = self.parse_int();
        }
        if (max >= 0 && min > max) || max > RE_DUP_MAX {
            return Err(REG_BADBR);
        }
        // the count-limit loop skips blanks and commas
        while self.at(1).is_some() && matches!(self.peek(), Some(' ') | Some(',')) {
            self.i += 1;
        }
        match self.peek() {
            None => return Err(REG_EBRACE),
            Some('}') => {}
            Some(c) if matches!(c, '+' | '-' | '#' | '~' | '<') || c.is_ascii_digit() => {
                // TRE's approximate matching parameters
                if !self.re[self.i..].contains(&'}') {
                    return Err(REG_EBRACE);
                }
                return Err("robigo: approximate regular expression matching is not supported");
            }
            Some(_) => {
                if !self.re[self.i..].contains(&'}') {
                    return Err(REG_EBRACE);
                }
                return Err(REG_BADBR);
            }
        }
        if self.i == start {
            return Err(REG_BADBR);
        }
        self.i += 1; // '}'
        let mut minimal = self.ungreedy;
        match self.peek() {
            Some('?') => {
                minimal = !self.ungreedy;
                self.i += 1;
            }
            Some('*') | Some('+') => return Err(REG_BADRPT),
            _ => {}
        }
        if min == 0 && max == 0 {
            return Ok(format!("(?:{}){{0}}", atom));
        }
        if min < 0 && max < 0 {
            min = 1;
            max = 1;
        }
        let min = min.max(0);
        let op = if max < 0 {
            format!("{{{},}}", min)
        } else if max == min {
            format!("{{{}}}", min)
        } else {
            format!("{{{},{}}}", min, max)
        };
        self.has_minimal |= minimal;
        Ok(format!("(?:{}){}{}", atom, op, if minimal { "?" } else { "" }))
    }

    // R: tre-parse.c:tre_parse_bracket and tre_parse_bracket_items
    fn parse_bracket(&mut self) -> TreRes<String> {
        let mut negate = false;
        if self.peek() == Some('^') {
            negate = true;
            self.i += 1;
        }
        let first = self.i;
        let mut items = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(REG_EBRACK) };
            if c == ']' && self.i > first {
                self.i += 1;
                break;
            }
            let end = self.re.len();
            if self.i + 2 < end && self.re[self.i + 1] == '-' && self.re[self.i + 2] != ']' {
                let (a, b) = (self.re[self.i], self.re[self.i + 2]);
                self.i += 3;
                if a > b {
                    return Err(REG_ERANGE);
                }
                esc(a, &mut items);
                items.push('-');
                esc(b, &mut items);
            } else if self.i + 1 < end && c == '[' && (self.re[self.i + 1] == '.' || self.re[self.i + 1] == '=') {
                return Err(REG_ECOLLATE);
            } else if self.i + 1 < end && c == '[' && self.re[self.i + 1] == ':' {
                let mut endp = self.i + 2;
                while endp < end && self.re[endp] != ':' {
                    endp += 1;
                }
                if endp == end {
                    return Err(REG_ECTYPE);
                }
                let name: String = self.re[self.i + 2..endp].iter().take(63).collect();
                let Some(cls) = tre_ctype(&name) else { return Err(REG_ECTYPE) };
                items.push_str(cls);
                self.i = endp + 2;
            } else {
                if c == '-' && self.at(1) != Some(']') && self.i != first {
                    return Err(REG_ERANGE);
                }
                esc(c, &mut items);
                self.i += 1;
            }
        }
        let class = format!("[{}{}]", if negate { "^" } else { "" }, items);
        if self.newline && negate {
            // REG_NEWLINE: a non-matching list never matches a newline
            return Ok(format!("[{}--\\n]", class));
        }
        Ok(if self.icase { format!("(?i:{})", class) } else { class })
    }

    // PARSE_ATOM
    fn parse_atom(&mut self) -> TreRes<String> {
        let Some(c) = self.peek() else { return Ok("(?:)".into()) };
        if self.literal {
            return self.parse_literal();
        }
        match c {
            '(' => {
                if self.at(1) == Some('?') {
                    return self.parse_extension();
                }
                self.depth += 1;
                self.i += 1;
                self.nsub += 1;
                let inner = self.parse_re()?;
                Ok(format!("({})", inner))
            }
            ')' if self.depth > 0 => Ok("(?:)".into()),
            '[' => {
                self.i += 1;
                self.parse_bracket()
            }
            '\\' => self.parse_escape(),
            '.' => {
                self.i += 1;
                Ok(if self.newline { r"[^\n]".into() } else { "(?s:.)".into() })
            }
            '^' => {
                self.i += 1;
                self.has_bol = true;
                Ok(if self.notbol {
                    NEVER.into()
                } else if self.newline {
                    "(?m:^)".into()
                } else {
                    "^".into()
                })
            }
            '$' => {
                self.i += 1;
                Ok(if self.newline { "(?m:$)".into() } else { "$".into() })
            }
            _ => self.parse_literal(),
        }
    }

    fn parse_literal(&mut self) -> TreRes<String> {
        if self.literal && self.peek() == Some('\\') && self.at(1) == Some('E') {
            self.literal = false;
            self.i += 2;
            return self.parse_piece();
        }
        let Some(c) = self.peek() else { return Ok("(?:)".into()) };
        if !self.literal && matches!(c, '*' | '|' | '{' | '+' | '?') {
            return Ok("(?:)".into());
        }
        self.i += 1;
        Ok(self.lit(c))
    }

    // "(?...)" extensions
    fn parse_extension(&mut self) -> TreRes<String> {
        self.i += 2;
        let (mut icase, mut newline, mut ungreedy) = (self.icase, self.newline, self.ungreedy);
        let mut bit = true;
        let mut group = false;
        loop {
            match self.peek() {
                Some('i') => {
                    icase = bit;
                    self.i += 1;
                }
                Some('n') => {
                    newline = bit;
                    self.i += 1;
                }
                Some('r') => self.i += 1,
                Some('U') => {
                    ungreedy = bit;
                    self.i += 1;
                }
                Some('-') => {
                    bit = false;
                    self.i += 1;
                }
                Some(':') => {
                    self.i += 1;
                    self.depth += 1;
                    group = true;
                    break;
                }
                Some('#') => {
                    while self.peek().is_some_and(|c| c != ')') {
                        self.i += 1;
                    }
                    if self.peek() == Some(')') {
                        self.i += 1;
                        break;
                    }
                    return Err(REG_BADPAT);
                }
                Some(')') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(REG_BADPAT),
            }
        }
        let saved = (self.icase, self.newline, self.ungreedy);
        self.icase = icase;
        self.newline = newline;
        self.ungreedy = ungreedy;
        let inner = self.parse_re()?;
        (self.icase, self.newline, self.ungreedy) = saved;
        let _ = group;
        Ok(format!("(?:{})", inner))
    }

    fn parse_escape(&mut self) -> TreRes<String> {
        // macros
        let next = self.at(1);
        let macro_ = match next {
            Some('t') => Some("\t".to_string()),
            Some('n') => Some("\n".to_string()),
            Some('r') => Some("\r".to_string()),
            Some('f') => Some("\x0c".to_string()),
            Some('a') => Some("\x07".to_string()),
            Some('e') => Some("\x1b".to_string()),
            Some('w') => Some("[[:alnum:]_]".to_string()),
            Some('W') => Some("[^[:alnum:]_]".to_string()),
            Some('s') => Some("[[:space:]]".to_string()),
            Some('S') => Some("[^[:space:]]".to_string()),
            Some('d') => Some("[[:digit:]]".to_string()),
            Some('D') => Some("[^[:digit:]]".to_string()),
            _ => None,
        };
        if let Some(m) = macro_ {
            let mut sub = TreParser {
                re: m.chars().collect(),
                i: 0,
                depth: 0,
                icase: self.icase,
                newline: self.newline,
                ungreedy: self.ungreedy,
                literal: false,
                notbol: self.notbol,
                nsub: 0,
                max_backref: 0,
                has_bol: false,
                has_minimal: false,
                has_lookaround: false,
            };
            let r = sub.parse_re()?;
            self.has_minimal |= sub.has_minimal;
            self.i += 2;
            return Ok(format!("(?:{})", r));
        }
        let Some(n) = next else { return Err(REG_EESCAPE) };
        if n == 'Q' {
            self.literal = true;
            self.i += 2;
            return self.parse_atom();
        }
        self.i += 2;
        Ok(match n {
            'b' => r"(?:\b|\A|\z)".into(),
            'B' => {
                // TRE: never at the start or end of the (sub)string
                self.has_lookaround = true;
                format!("(?:(?<={w})(?={w})|(?<={nw})(?={nw}))", w = TRE_WORD, nw = TRE_NONWORD)
            }
            '<' => r"\<".into(),
            '>' => r"\>".into(),
            'x' => {
                if self.peek() != Some('{') && self.peek().is_some() {
                    let mut tmp = String::new();
                    for _ in 0..2 {
                        if let Some(c) = self.peek().filter(|c| c.is_ascii_hexdigit()) {
                            tmp.push(c);
                            self.i += 1;
                        }
                    }
                    let v = u32::from_str_radix(&tmp, 16).unwrap_or(0);
                    self.lit(char::from_u32(v).unwrap_or('\0'))
                } else if self.peek().is_some() {
                    self.i += 1;
                    let mut tmp = String::new();
                    loop {
                        match self.peek() {
                            Some('}') => break,
                            Some(c) if c.is_ascii_hexdigit() => {
                                tmp.push(c);
                                self.i += 1;
                            }
                            _ => return Err(REG_EBRACE),
                        }
                    }
                    self.i += 1;
                    let v = u32::from_str_radix(&tmp, 16).unwrap_or(0);
                    self.lit(char::from_u32(v).unwrap_or('\u{FFFD}'))
                } else {
                    self.lit('x')
                }
            }
            d if d.is_ascii_digit() => {
                let v = d as usize - '0' as usize;
                self.max_backref = self.max_backref.max(v);
                format!("\\{}", v)
            }
            c => self.lit(c),
        })
    }
}

/// Translate a TRE extended regular expression. `literal` is REG_LITERAL
/// (regexec with fixed = TRUE).
pub fn tre_translate(pat: &str, icase: bool, notbol: bool, literal: bool) -> TreRes<TreOut> {
    if literal {
        let mut p = esc_str(pat);
        if icase {
            p = format!("(?i:{})", p);
        }
        return Ok(TreOut { pattern: p, nsub: 0, has_backref: false, has_bol: false, has_minimal: false, has_lookaround: false });
    }
    let mut p = TreParser {
        re: pat.chars().collect(),
        i: 0,
        depth: 0,
        icase,
        newline: false,
        ungreedy: false,
        literal: false,
        notbol,
        nsub: 0,
        max_backref: 0,
        has_bol: false,
        has_minimal: false,
        has_lookaround: false,
    };
    let out = p.parse_re()?;
    if p.depth > 0 {
        return Err(REG_EPAREN);
    }
    if p.max_backref > p.nsub {
        return Err(REG_ESUBREG);
    }
    Ok(TreOut {
        pattern: out,
        nsub: p.nsub,
        has_backref: p.max_backref > 0,
        has_bol: p.has_bol,
        has_minimal: p.has_minimal,
        has_lookaround: p.has_lookaround,
    })
}

// ======================================================================
// PCRE syntax -> fancy-regex syntax

const W_ASCII: &str = "[0-9A-Za-z_]";
const H_SPACE: &str = r"\t\x{20}\x{A0}\x{1680}\x{180E}\x{2000}-\x{200A}\x{202F}\x{205F}\x{3000}";
const V_SPACE: &str = r"\n\x{B}\x{C}\r\x{85}\x{2028}\x{2029}";

#[derive(Debug)]
pub struct PcreErr {
    msg: String,
    offset: usize,
}

fn perr(msg: &str, offset: usize) -> PcreErr {
    PcreErr { msg: msg.to_string(), offset }
}

/// Translate a PCRE pattern. `ucp` enables Unicode classes (PCRE2_UCP).
fn pcre_translate(pat: &str) -> Result<String, PcreErr> {
    let b: Vec<char> = pat.chars().collect();
    // byte offsets of each char, for error positions
    let boff: Vec<usize> = pat.char_indices().map(|(i, _)| i).chain(std::iter::once(pat.len())).collect();
    let mut i = 0;
    let mut ucp = false;
    // leading verbs
    loop {
        let rest: String = b[i..].iter().collect();
        if rest.starts_with("(*UCP)") {
            ucp = true;
            i += 6;
        } else if rest.starts_with("(*UTF8)") {
            i += 7;
        } else if rest.starts_with("(*UTF)") {
            i += 6;
        } else {
            break;
        }
    }
    let wclass = |neg: bool| -> String {
        if ucp {
            if neg { r"\W".into() } else { r"\w".into() }
        } else if neg {
            "[^0-9A-Za-z_]".into()
        } else {
            W_ASCII.into()
        }
    };
    let word_b = |neg: bool| -> String {
        if ucp {
            if neg { r"\B".into() } else { r"\b".into() }
        } else if neg {
            format!("(?:(?<={w})(?={w})|(?<!{w})(?!{w}))", w = W_ASCII)
        } else {
            format!("(?:(?<={w})(?!{w})|(?<!{w})(?={w}))", w = W_ASCII)
        }
    };
    let multiline = {
        // crude: a (?m) or (?…m…: flag anywhere turns `$` into a line anchor
        let mut m = false;
        let mut k = 0;
        while k + 1 < b.len() {
            if b[k] == '(' && b[k + 1] == '?' {
                let mut j = k + 2;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    if b[j] == 'm' {
                        m = true;
                    }
                    j += 1;
                }
            }
            k += 1;
        }
        m
    };
    let mut out = String::new();
    // start (in `out`) of the last atom, for possessive quantifiers
    let mut atom_start: Option<usize> = None;
    let mut group_starts: Vec<usize> = Vec::new();
    let mut in_class = false;
    let mut class_start_i = 0usize;
    // start (in `out`) of the open bracket expression
    let mut class_out_start = 0usize;
    let mut depth = 0usize;
    let mut last_was_quant = false;
    while i < b.len() {
        let c = b[i];
        if in_class {
            match c {
                ']' if i > class_start_i => {
                    out.push(']');
                    in_class = false;
                    i += 1;
                    // a bracket expression is a repeatable atom
                    atom_start = Some(class_out_start);
                    last_was_quant = false;
                }
                '[' if b.get(i + 1) == Some(&':') => {
                    // POSIX class [:name:]
                    let mut j = i + 2;
                    while j < b.len() && b[j] != ']' {
                        j += 1;
                    }
                    let name: String = b[i..(j + 1).min(b.len())].iter().collect();
                    out.push_str(&name);
                    i = j + 1;
                }
                '[' => {
                    out.push_str(r"\[");
                    i += 1;
                }
                '\\' => {
                    let Some(&n) = b.get(i + 1) else {
                        return Err(perr("\\ at end of pattern", pat.len()));
                    };
                    i += 2;
                    match n {
                        'w' => out.push_str(if ucp { r"\w" } else { "0-9A-Za-z_" }),
                        'W' => out.push_str(if ucp { r"\W" } else { "[^0-9A-Za-z_]" }),
                        'd' => out.push_str(if ucp { r"\d" } else { "0-9" }),
                        'D' => out.push_str(if ucp { r"\D" } else { "[^0-9]" }),
                        's' => out.push_str(if ucp { r"\s" } else { r"\t\n\x{B}\x{C}\r\x{20}" }),
                        'S' => out.push_str(if ucp { r"\S" } else { r"[^\t\n\x{B}\x{C}\r\x{20}]" }),
                        'h' => out.push_str(H_SPACE),
                        'H' => out.push_str(&format!("[^{}]", H_SPACE)),
                        'v' => out.push_str(V_SPACE),
                        'V' => out.push_str(&format!("[^{}]", V_SPACE)),
                        'e' => out.push_str(r"\x{1B}"),
                        'a' => out.push_str(r"\x{7}"),
                        'b' => out.push_str(r"\x{8}"),
                        'n' | 't' | 'r' | 'f' => {
                            out.push('\\');
                            out.push(n);
                        }
                        'x' | 'p' | 'P' => {
                            out.push('\\');
                            out.push(n);
                        }
                        c if c.is_ascii_alphanumeric() => {
                            out.push('\\');
                            out.push(c);
                        }
                        c => esc(c, &mut out),
                    }
                }
                '&' | '~' | '-' if b.get(i + 1) == Some(&c) => {
                    // regex-syntax set operators; PCRE literal
                    esc(c, &mut out);
                    i += 1;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
            continue;
        }
        let this_start = out.len();
        let mut is_atom = true;
        match c {
            '\\' => {
                let Some(&n) = b.get(i + 1) else {
                    return Err(perr("\\ at end of pattern", pat.len()));
                };
                i += 2;
                match n {
                    'w' => out.push_str(&wclass(false)),
                    'W' => out.push_str(&wclass(true)),
                    'd' => out.push_str(if ucp { r"\d" } else { "[0-9]" }),
                    'D' => out.push_str(if ucp { r"\D" } else { "[^0-9]" }),
                    's' => out.push_str(if ucp { r"\s" } else { r"[\t\n\x{B}\x{C}\r\x{20}]" }),
                    'S' => out.push_str(if ucp { r"\S" } else { r"[^\t\n\x{B}\x{C}\r\x{20}]" }),
                    'b' => {
                        out.push_str(&word_b(false));
                        is_atom = false;
                    }
                    'B' => {
                        out.push_str(&word_b(true));
                        is_atom = false;
                    }
                    'h' => out.push_str(&format!("[{}]", H_SPACE)),
                    'H' => out.push_str(&format!("[^{}]", H_SPACE)),
                    'v' => out.push_str(&format!("[{}]", V_SPACE)),
                    'V' => out.push_str(&format!("[^{}]", V_SPACE)),
                    'R' => out.push_str(&format!("(?:\\r\\n|[{}])", V_SPACE)),
                    'N' => out.push_str(r"[^\n]"),
                    'Z' => {
                        out.push_str(r"(?=\n?\z)");
                        is_atom = false;
                    }
                    'z' | 'A' | 'G' | 'K' => {
                        out.push('\\');
                        out.push(n);
                        is_atom = false;
                    }
                    'Q' => {
                        let mut lit = String::new();
                        while i < b.len() {
                            if b[i] == '\\' && b.get(i + 1) == Some(&'E') {
                                i += 2;
                                break;
                            }
                            lit.push(b[i]);
                            i += 1;
                        }
                        if lit.is_empty() {
                            is_atom = false;
                        }
                        out.push_str(&esc_str(&lit));
                    }
                    'E' => is_atom = false,
                    'e' => out.push_str(r"\x{1B}"),
                    'a' => out.push_str(r"\x{7}"),
                    'c' => {
                        let Some(&x) = b.get(i) else {
                            return Err(perr("\\c at end of pattern", pat.len()));
                        };
                        i += 1;
                        let v = (x.to_ascii_uppercase() as u32) ^ 0x40;
                        out.push_str(&format!("\\x{{{:X}}}", v));
                    }
                    'o' if b.get(i) == Some(&'{') => {
                        let mut j = i + 1;
                        let mut v = 0u32;
                        while j < b.len() && b[j] != '}' {
                            v = v * 8 + b[j].to_digit(8).unwrap_or(0);
                            j += 1;
                        }
                        i = j + 1;
                        out.push_str(&format!("\\x{{{:X}}}", v));
                    }
                    '0' => {
                        let mut v = 0u32;
                        let mut k = 0;
                        while k < 2 && b.get(i).is_some_and(|c| c.is_digit(8)) {
                            v = v * 8 + b[i].to_digit(8).unwrap();
                            i += 1;
                            k += 1;
                        }
                        out.push_str(&format!("\\x{{{:X}}}", v));
                    }
                    'g' => {
                        // \g{n}, \gn, \g{-n}: back references
                        let mut j = i;
                        let braced = b.get(j) == Some(&'{');
                        if braced {
                            j += 1;
                        }
                        let mut num = String::new();
                        while j < b.len() && (b[j].is_ascii_digit() || (b[j] == '-' && num.is_empty())) {
                            num.push(b[j]);
                            j += 1;
                        }
                        if braced {
                            j += 1;
                        }
                        i = j;
                        out.push('\\');
                        out.push_str(num.trim_start_matches('-'));
                    }
                    c if c.is_ascii_digit() => {
                        out.push('\\');
                        out.push(c);
                        while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                            out.push(b[i]);
                            i += 1;
                        }
                    }
                    'x' | 'p' | 'P' | 'k' | 'n' | 't' | 'r' | 'f' => {
                        out.push('\\');
                        out.push(n);
                        // copy the argument of \x{..} / \p{..} / \k<..>
                        if matches!(b.get(i), Some('{') | Some('<') | Some('\'')) {
                            let close = match b[i] {
                                '{' => '}',
                                '<' => '>',
                                _ => '\'',
                            };
                            while i < b.len() {
                                out.push(b[i]);
                                i += 1;
                                if b[i - 1] == close {
                                    break;
                                }
                            }
                        }
                    }
                    c if c.is_ascii_alphanumeric() => {
                        return Err(perr("unrecognized character follows \\", boff[i]));
                    }
                    c => esc(c, &mut out),
                }
            }
            '[' => {
                in_class = true;
                class_out_start = this_start;
                out.push('[');
                i += 1;
                if b.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                class_start_i = i;
                if b.get(i) == Some(&']') {
                    out.push_str(r"\]");
                    i += 1;
                    class_start_i = i - 1;
                }
                continue;
            }
            '(' => {
                let rest: String = b[i..b.len().min(i + 3)].iter().collect();
                if rest.starts_with("(?#") {
                    while i < b.len() && b[i] != ')' {
                        i += 1;
                    }
                    if i == b.len() {
                        return Err(perr("missing ) after (?# comment", pat.len()));
                    }
                    i += 1;
                    continue;
                }
                if rest.starts_with("(*") {
                    return Err(perr("robigo: PCRE (*VERB) syntax is not supported", boff[i]));
                }
                group_starts.push(out.len());
                depth += 1;
                out.push('(');
                i += 1;
                if b.get(i) == Some(&'?') {
                    // copy the group prefix verbatim
                    out.push('?');
                    i += 1;
                    let copy_until = |i: &mut usize, out: &mut String, close: char| {
                        while *i < b.len() {
                            out.push(b[*i]);
                            *i += 1;
                            if b[*i - 1] == close {
                                break;
                            }
                        }
                    };
                    match b.get(i).copied() {
                        Some(':' | '=' | '!' | '>' | '|') => {
                            out.push(b[i]);
                            i += 1;
                        }
                        Some('<') if matches!(b.get(i + 1), Some('=') | Some('!')) => {
                            out.push('<');
                            out.push(b[i + 1]);
                            i += 2;
                        }
                        Some('<') => copy_until(&mut i, &mut out, '>'),
                        Some('\'') => {
                            // (?'name' -> (?<name>
                            out.push('<');
                            i += 1;
                            while i < b.len() && b[i] != '\'' {
                                out.push(b[i]);
                                i += 1;
                            }
                            out.push('>');
                            i += 1;
                        }
                        Some('P') if b.get(i + 1) == Some(&'<') => copy_until(&mut i, &mut out, '>'),
                        Some('P') if b.get(i + 1) == Some(&'=') => {
                            // (?P=name): a complete back reference
                            copy_until(&mut i, &mut out, ')');
                            depth -= 1;
                            group_starts.pop();
                            atom_start = Some(this_start);
                            last_was_quant = false;
                            continue;
                        }
                        Some('(') => {
                            // conditional: copy the condition
                            copy_until(&mut i, &mut out, ')');
                        }
                        Some(c) if c.is_ascii_alphabetic() || c == '-' || c == '^' => {
                            // option setting: (?i) or (?i:
                            while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == '-' || b[i] == '^') {
                                if b[i] != '^' {
                                    out.push(b[i]);
                                }
                                i += 1;
                            }
                            match b.get(i) {
                                Some(')') => {
                                    out.push(')');
                                    i += 1;
                                    depth -= 1;
                                    group_starts.pop();
                                    atom_start = None;
                                    last_was_quant = false;
                                    continue;
                                }
                                Some(':') => {
                                    out.push(':');
                                    i += 1;
                                }
                                _ => return Err(perr("unrecognized character after (? or (?-", boff[i.min(b.len())])),
                            }
                        }
                        Some(c) if c == 'R' || c == '&' || c == '+' || c.is_ascii_digit() => {
                            return Err(perr("robigo: PCRE recursion is not supported", boff[i]));
                        }
                        _ => return Err(perr("unrecognized character after (? or (?-", boff[i.min(b.len())])),
                    }
                }
                last_was_quant = false;
                atom_start = None;
                continue;
            }
            ')' => {
                if depth == 0 {
                    return Err(perr("unmatched closing parenthesis", boff[i]));
                }
                depth -= 1;
                out.push(')');
                i += 1;
                atom_start = group_starts.pop();
                last_was_quant = false;
                continue;
            }
            '$' => {
                out.push_str(if multiline { "$" } else { r"(?=\n?\z)" });
                i += 1;
                is_atom = false;
            }
            '^' | '|' => {
                out.push(c);
                i += 1;
                is_atom = false;
            }
            '*' | '+' | '?' | '{' => {
                // quantifier
                let qstart = i;
                if c == '{' {
                    // only a quantifier if it has the form {n}, {n,}, {n,m}
                    let mut j = i + 1;
                    let mut ok = false;
                    let mut digits = 0;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                        digits += 1;
                    }
                    if digits > 0 {
                        if b.get(j) == Some(&'}') {
                            ok = true;
                        } else if b.get(j) == Some(&',') {
                            j += 1;
                            while j < b.len() && b[j].is_ascii_digit() {
                                j += 1;
                            }
                            ok = b.get(j) == Some(&'}');
                        }
                    }
                    if !ok {
                        out.push_str(r"\{");
                        i += 1;
                        atom_start = Some(this_start);
                        last_was_quant = false;
                        continue;
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
                let q: String = b[qstart..i].iter().collect();
                let Some(astart) = atom_start.filter(|_| !last_was_quant) else {
                    if last_was_quant && c == '?' {
                        // lazy modifier already handled; PCRE allows `??` etc.
                        out.push('?');
                        continue;
                    }
                    return Err(perr("quantifier does not follow a repeatable item", boff[qstart]));
                };
                match b.get(i) {
                    Some('+') => {
                        // possessive: atomic group around the quantified atom
                        i += 1;
                        let atom = out.split_off(astart);
                        out.push_str(&format!("(?>{}{})", atom, q));
                    }
                    Some('?') => {
                        i += 1;
                        out.push_str(&q);
                        out.push('?');
                    }
                    _ => out.push_str(&q),
                }
                last_was_quant = true;
                continue;
            }
            '.' => {
                out.push('.');
                i += 1;
            }
            c => {
                if c.is_ascii_alphanumeric() || c == '_' || c == ' ' || !c.is_ascii() {
                    out.push(c);
                } else {
                    esc(c, &mut out);
                }
                i += 1;
            }
        }
        atom_start = if is_atom { Some(this_start) } else { None };
        last_was_quant = false;
    }
    if in_class {
        return Err(perr("missing terminating ] for character class", pat.len()));
    }
    if depth > 0 {
        return Err(perr("missing closing parenthesis", pat.len()));
    }
    Ok(out)
}

// ======================================================================
// Compiled patterns

/// A TRE program: leftmost-longest matching via regex-automata, or
/// fancy-regex for patterns with back-references.
enum TreProg {
    Auto {
        meta: MetaRegex,
        dfa: Option<DFA>,
        cache: RefCell<Option<DfaCache>>,
        cap: Option<MetaRegex>,
    },
    Fancy(fancy_regex::Regex),
}

type Spans = Vec<Option<(usize, usize)>>;

impl TreProg {
    fn new(t: &TreOut, want_caps: bool) -> Result<TreProg, String> {
        if t.has_backref || t.has_lookaround {
            return fancy_regex::Regex::new(&t.pattern).map(TreProg::Fancy).map_err(|e| e.to_string());
        }
        let meta = MetaRegex::new(&t.pattern).map_err(|e| e.to_string())?;
        // TRE honours minimal repetitions, so only extend to the longest
        // match when there are none.
        let dfa = if t.has_minimal {
            None
        } else {
            DFA::builder()
                .configure(DFA::config().match_kind(MatchKind::All).unicode_word_boundary(true))
                .build(&t.pattern)
                .ok()
        };
        let cap = if want_caps && t.nsub > 0 {
            MetaRegex::new(&format!(r"(?:{})\z", t.pattern)).ok()
        } else {
            None
        };
        Ok(TreProg::Auto { meta, dfa, cache: RefCell::new(None), cap })
    }

    /// Leftmost-longest match in `s` (a fresh string).
    fn exec(&self, s: &str, groups: bool) -> Option<Spans> {
        match self {
            TreProg::Fancy(re) => {
                let caps = re.captures(s).ok()??;
                Some((0..caps.len()).map(|i| caps.get(i).map(|m| (m.start(), m.end()))).collect())
            }
            TreProg::Auto { meta, dfa, cache, cap } => {
                let m = meta.find(s)?;
                let s0 = m.start();
                let mut e = m.end();
                if let Some(dfa) = dfa {
                    let mut c = cache.borrow_mut();
                    let c = c.get_or_insert_with(|| dfa.create_cache());
                    let input = Input::new(s).range(s0..).anchored(Anchored::Yes);
                    if let Ok(Some(hm)) = dfa.try_search_fwd(c, &input) {
                        if hm.offset() > e {
                            e = hm.offset();
                        }
                    }
                }
                let mut spans: Spans = vec![Some((s0, e))];
                if groups {
                    let ngroups = meta.captures_len();
                    let mut got = false;
                    if let Some(cap) = cap {
                        let mut caps = cap.create_captures();
                        let input = Input::new(&s[..e]).range(s0..e).anchored(Anchored::Yes);
                        cap.search_captures(&input, &mut caps);
                        if caps.is_match() {
                            spans = (0..ngroups).map(|i| caps.get_group(i).map(|sp| (sp.start, sp.end))).collect();
                            spans[0] = Some((s0, e));
                            got = true;
                        }
                    }
                    if !got {
                        let mut caps = meta.create_captures();
                        let input = Input::new(s).range(s0..).anchored(Anchored::Yes);
                        meta.search_captures(&input, &mut caps);
                        spans = (0..ngroups).map(|i| caps.get_group(i).map(|sp| (sp.start, sp.end))).collect();
                        spans[0] = Some((s0, e));
                    }
                }
                Some(spans)
            }
        }
    }
}

enum Kind {
    Fixed(String),
    Tre { std: TreProg, notbol: Option<TreProg> },
    Pcre(fancy_regex::Regex),
}

struct Re {
    kind: Kind,
    /// Number of capture groups (without the whole match).
    nsub: usize,
}

/// How to start a search at a byte offset.
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    /// Search `hay[start..]` as a fresh string; `notbol` is REG_NOTBOL.
    Fresh { notbol: bool },
    /// Search from `start` with the whole string as context (PCRE).
    Full,
}

impl Re {
    /// R: grep.c:reg_report (TRE compile errors).
    fn tre_error(interp: &mut Interp, pat: &str, reason: &str) -> Flow {
        if reason.starts_with("robigo:") {
            return Flow::error(reason.to_string());
        }
        if let Err(f) = interp.warning(format!("TRE pattern compilation error '{}'", reason)) {
            return f;
        }
        Flow::error(format!("invalid regular expression '{}', reason '{}'", pat, reason))
    }

    fn pcre_error(interp: &mut Interp, pat: &str, msg: &str, offset: usize) -> Flow {
        if msg.starts_with("robigo:") {
            return Flow::error(msg.to_string());
        }
        let at = pat.get(offset..).unwrap_or("");
        if let Err(f) = interp.warning(format!("PCRE pattern compilation error\n\t'{}'\n\tat '{}'", msg, at)) {
            return f;
        }
        Flow::error(format!("invalid regular expression '{}'", pat))
    }

    fn compile(interp: &mut Interp, pat: &str, fixed: bool, perl: bool, icase: bool, want_caps: bool) -> RResult<Re> {
        if fixed {
            return Ok(Re { kind: Kind::Fixed(pat.to_string()), nsub: 0 });
        }
        if perl {
            let t = match pcre_translate(pat) {
                Ok(t) => t,
                Err(e) => return Err(Re::pcre_error(interp, pat, &e.msg, e.offset)),
            };
            let t = if icase { format!("(?i){}", t) } else { t };
            let re = match fancy_regex::Regex::new(&t) {
                Ok(r) => r,
                Err(e) => {
                    let msg = pcre_message(&e);
                    return Err(Re::pcre_error(interp, pat, &msg, 0));
                }
            };
            let nsub = re.captures_len() - 1;
            return Ok(Re { kind: Kind::Pcre(re), nsub });
        }
        let t = tre_translate(pat, icase, false, false).map_err(|r| Re::tre_error(interp, pat, r))?;
        let std = TreProg::new(&t, want_caps).map_err(|e| unsupported(pat, &e))?;
        let notbol = if t.has_bol {
            let t2 = tre_translate(pat, icase, true, false).map_err(|r| Re::tre_error(interp, pat, r))?;
            Some(TreProg::new(&t2, want_caps).map_err(|e| unsupported(pat, &e))?)
        } else {
            None
        };
        Ok(Re { kind: Kind::Tre { std, notbol }, nsub: t.nsub })
    }

    /// Capture names for perl = TRUE ("" for unnamed groups).
    fn capture_names(&self) -> Vec<RStr> {
        match &self.kind {
            Kind::Pcre(re) => re.capture_names().skip(1).map(|n| Some(Rc::from(n.unwrap_or("")))).collect(),
            _ => vec![Some(Rc::from("")); self.nsub],
        }
    }

    /// First match at or after byte `start`; spans are absolute byte
    /// offsets, index 0 being the whole match.
    fn exec(&self, hay: &str, start: usize, ctx: Ctx, groups: bool) -> Option<Spans> {
        match &self.kind {
            Kind::Fixed(p) => {
                let i = hay[start..].find(p.as_str())?;
                Some(vec![Some((start + i, start + i + p.len()))])
            }
            Kind::Tre { std, notbol } => {
                let prog = match (ctx, notbol) {
                    (Ctx::Fresh { notbol: true }, Some(nb)) => nb,
                    _ => std,
                };
                let spans = prog.exec(&hay[start..], groups)?;
                Some(spans.into_iter().map(|o| o.map(|(a, b)| (a + start, b + start))).collect())
            }
            Kind::Pcre(re) => {
                let (sub, base) = match ctx {
                    Ctx::Fresh { .. } => (&hay[start..], start),
                    Ctx::Full => (hay, 0),
                };
                let from = if base == 0 { start } else { 0 };
                if groups {
                    let caps = re.captures_from_pos(sub, from).ok()??;
                    Some((0..caps.len()).map(|i| caps.get(i).map(|m| (m.start() + base, m.end() + base))).collect())
                } else {
                    let m = re.find_from_pos(sub, from).ok()??;
                    Some(vec![Some((m.start() + base, m.end() + base))])
                }
            }
        }
    }
}

fn unsupported(pat: &str, e: &str) -> Flow {
    Flow::error(format!("robigo: regular expression '{}' is not supported: {}", pat, e.lines().next().unwrap_or("")))
}

/// PCRE2-style messages for fancy-regex compile errors.
fn pcre_message(e: &fancy_regex::Error) -> String {
    use fancy_regex::{CompileError, Error, ParseError};
    match e {
        Error::ParseError(_, pe) => match pe {
            ParseError::UnclosedOpenParen => "missing closing parenthesis".into(),
            ParseError::InvalidClass => "missing terminating ] for character class".into(),
            ParseError::TargetNotRepeatable => "quantifier does not follow a repeatable item".into(),
            ParseError::TrailingBackslash => "\\ at end of pattern".into(),
            ParseError::InvalidBackref => "reference to non-existent subpattern".into(),
            other => other.to_string(),
        },
        Error::CompileError(ce) => match ce {
            CompileError::LookBehindNotConst => "length of lookbehind assertion is not limited".into(),
            CompileError::InvalidBackref(_) => "reference to non-existent subpattern".into(),
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}

// ======================================================================
// helpers

fn nchars(s: &str) -> usize {
    s.chars().count()
}

fn char_len_at(s: &str, i: usize) -> usize {
    s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

fn only_ascii(v: &[RStr]) -> bool {
    v.iter().all(|e| e.as_deref().is_none_or(|s| s.is_ascii()))
}

fn str_vec(v: &Value) -> Option<&Vector<RStr>> {
    match v {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

struct Opts {
    icase: bool,
    perl: bool,
    fixed: bool,
}

/// The common checks of do_grep/do_gsub/do_regexpr on `pattern`.
fn pattern_arg(interp: &mut Interp, pat: &Value) -> RResult<RStr> {
    let p = match pat {
        Value::Str(p) if !p.data.is_empty() => p,
        _ => return Err(Flow::error("invalid 'pattern' argument")),
    };
    if p.data.len() > 1 {
        interp.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    Ok(p.data[0].clone())
}

fn fix_opts(interp: &mut Interp, o: &mut Opts) -> RResult<()> {
    if o.fixed && o.icase {
        interp.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if o.fixed && o.perl {
        interp.warning("argument 'perl = TRUE' will be ignored")?;
        o.perl = false;
    }
    Ok(())
}

// ======================================================================
// strsplit

// R: grep.c:do_strsplit
fn do_strsplit(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "strsplit")?;
    let x = &args[0].value;
    let tok = &args[1].value;
    let fixed = as_bool2(&args[2].value, call)?;
    let mut perl = as_bool2(&args[3].value, call)?;
    let _use_bytes = as_bool2(&args[4].value, call)?;
    if fixed && perl {
        interp.warning("argument 'perl = TRUE' will be ignored")?;
        perl = false;
    }
    let (Some(xv), Some(tv)) = (str_vec(x), str_vec(tok)) else {
        return Err(Flow::error("non-character argument"));
    };
    let len = xv.data.len();
    let toks: Vec<RStr> = if tv.data.is_empty() { vec![Some(Rc::from(""))] } else { tv.data.clone() };
    let tlen = toks.len();
    let mut ans: Vec<Value> = vec![Value::Null; len];
    for (itok, this) in toks.iter().enumerate() {
        let idx: Vec<usize> = (itok..len).step_by(tlen).collect();
        match this {
            None => {
                for i in idx {
                    ans[i] = Value::strs(vec![xv.data[i].clone()]);
                }
            }
            Some(split) if split.is_empty() => {
                for i in idx {
                    ans[i] = match &xv.data[i] {
                        None => Value::str_na(),
                        Some(s) => Value::strs(s.chars().map(|c| Some(Rc::from(c.to_string().as_str()))).collect()),
                    };
                }
            }
            Some(split) if fixed => {
                for i in idx {
                    ans[i] = match &xv.data[i] {
                        None => Value::str_na(),
                        Some(s) => {
                            let mut v: Vec<RStr> = Vec::new();
                            let mut rest: &str = s;
                            while let Some(p) = rest.find(&**split) {
                                v.push(Some(Rc::from(&rest[..p])));
                                rest = &rest[p + split.len()..];
                            }
                            if !rest.is_empty() {
                                v.push(Some(Rc::from(rest)));
                            }
                            Value::strs(v)
                        }
                    };
                }
            }
            Some(split) => {
                let re = Re::compile(interp, split, false, perl, false, false)?;
                for i in idx {
                    ans[i] = match &xv.data[i] {
                        None => Value::str_na(),
                        Some(s) => {
                            let mut v: Vec<RStr> = Vec::new();
                            let mut bufp = 0usize;
                            if !s.is_empty() {
                                while let Some(m) = re.exec(s, bufp, Ctx::Fresh { notbol: false }, false) {
                                    let (so, eo) = m[0].unwrap();
                                    if eo > bufp {
                                        v.push(Some(Rc::from(&s[bufp..so])));
                                        bufp = eo;
                                    } else {
                                        let cl = char_len_at(s, bufp);
                                        v.push(Some(Rc::from(&s[bufp..bufp + cl])));
                                        bufp += cl;
                                    }
                                    if bufp >= s.len() {
                                        break;
                                    }
                                }
                            }
                            if bufp < s.len() {
                                v.push(Some(Rc::from(&s[bufp..])));
                            }
                            Value::strs(v)
                        }
                    };
                }
            }
        }
    }
    let mut out = Value::list(ans);
    if let Some(nm) = x.get_attr(s::NAMES) {
        out.set_attr_raw(s::NAMES, nm);
    }
    Ok(out)
}

// ======================================================================
// grep, grepl

fn grep_impl(interp: &mut Interp, call: &Value, args: Vec<Arg>, grepl: bool) -> RResult<Value> {
    check_arity_internal(&args, 8, if grepl { "grepl" } else { "grep" })?;
    let pat = &args[0].value;
    let text = &args[1].value;
    let mut o = Opts {
        icase: as_bool2(&args[2].value, call)?,
        perl: false,
        fixed: false,
    };
    let value = as_bool2(&args[3].value, call)?;
    o.perl = as_bool2(&args[4].value, call)?;
    o.fixed = as_bool2(&args[5].value, call)?;
    let _use_bytes = as_bool2(&args[6].value, call)?;
    let invert = as_bool2(&args[7].value, call)?;
    fix_opts(interp, &mut o)?;
    let p = pattern_arg(interp, pat)?;
    let Some(tv) = str_vec(text) else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let n = tv.data.len();
    let Some(p) = p else {
        if value {
            let mut ans = Value::strs(vec![None; n]);
            if let Some(nm) = text.get_attr(s::NAMES) {
                ans.set_attr_raw(s::NAMES, nm);
            }
            return Ok(ans);
        } else if grepl {
            return Ok(Value::lgls(vec![NA_LOGICAL; n]));
        } else {
            return Ok(Value::ints(vec![NA_INTEGER; n]));
        }
    };
    let re = Re::compile(interp, &p, o.fixed, o.perl, o.icase && !o.fixed, false)?;
    let ind: Vec<bool> = tv
        .data
        .iter()
        .map(|e| match e {
            None => false,
            Some(s) => re.exec(s, 0, Ctx::Fresh { notbol: false }, false).is_some(),
        })
        .collect();
    if grepl {
        return Ok(Value::lgls(ind.iter().map(|&b| b as Lgl).collect()));
    }
    let sel: Vec<usize> = (0..n).filter(|&i| invert ^ ind[i]).collect();
    if value {
        let mut ans = Value::strs(sel.iter().map(|&i| tv.data[i].clone()).collect());
        if let Some(Value::Str(nm)) = text.get_attr(s::NAMES) {
            ans.set_attr_raw(s::NAMES, Value::strs(sel.iter().map(|&i| nm.data[i].clone()).collect()));
        }
        Ok(ans)
    } else {
        Ok(Value::ints(sel.iter().map(|&i| i as i32 + 1).collect()))
    }
}

fn do_grep(i: &mut Interp, c: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    grep_impl(i, c, a, false)
}

fn do_grepl(i: &mut Interp, c: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    grep_impl(i, c, a, true)
}

// ======================================================================
// sub, gsub

/// Count the `\1`..`\9` references in a replacement (R: count_subs).
fn has_backrefs(repl: &str) -> bool {
    let b = repl.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            if i + 1 < b.len() && (b'1'..=b'9').contains(&b[i + 1]) {
                return true;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    false
}

/// R: grep.c:string_adj (TRE) and R_pcre_string_adj (`perl`, with \U \L \E).
fn string_adj(out: &mut String, s: &str, repl: &str, m: &Spans, perl: bool) {
    let r: Vec<char> = repl.chars().collect();
    let mut p = 0;
    let (mut upper, mut lower) = (false, false);
    while p < r.len() {
        if r[p] == '\\' {
            if p + 1 < r.len() && ('1'..='9').contains(&r[p + 1]) {
                let k = r[p + 1] as usize - '0' as usize;
                if let Some(Some((a, b))) = m.get(k) {
                    let g = &s[*a..*b];
                    if perl && (upper || lower) {
                        for c in g.chars() {
                            out.push(if upper { simple_upper(c) } else { simple_lower(c) });
                        }
                    } else {
                        out.push_str(g);
                    }
                }
                p += 2;
            } else if perl && p + 1 < r.len() && r[p + 1] == 'U' {
                p += 2;
                upper = true;
                lower = false;
            } else if perl && p + 1 < r.len() && r[p + 1] == 'L' {
                p += 2;
                upper = false;
                lower = true;
            } else if perl && p + 1 < r.len() && r[p + 1] == 'E' {
                p += 2;
                upper = false;
                lower = false;
            } else if p + 1 >= r.len() {
                p += 1;
            } else {
                out.push(r[p + 1]);
                p += 2;
            }
        } else {
            out.push(r[p]);
            p += 1;
        }
    }
}

// R: grep.c:do_gsub
fn gsub_impl(interp: &mut Interp, call: &Value, args: Vec<Arg>, global: bool) -> RResult<Value> {
    check_arity_internal(&args, 7, if global { "gsub" } else { "sub" })?;
    let pat = &args[0].value;
    let rep = &args[1].value;
    let text = &args[2].value;
    let mut o = Opts {
        icase: as_bool2(&args[3].value, call)?,
        perl: as_bool2(&args[4].value, call)?,
        fixed: as_bool2(&args[5].value, call)?,
    };
    let _use_bytes = as_bool2(&args[6].value, call)?;
    fix_opts(interp, &mut o)?;
    let p = pattern_arg(interp, pat)?;
    let r = match rep {
        Value::Str(r) if !r.data.is_empty() => r.clone(),
        _ => return Err(Flow::error("invalid 'replacement' argument")),
    };
    if r.data.len() > 1 {
        interp.warning("argument 'replacement' has length > 1 and only the first element will be used")?;
    }
    let Some(tv) = str_vec(text) else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let n = tv.data.len();
    let out: Vec<RStr> = match p {
        None => vec![None; n],
        Some(p) => {
            let repl = r.data[0].clone();
            let srep: &str = repl.as_deref().unwrap_or("NA");
            if o.fixed && p.is_empty() {
                return Err(Flow::error("zero-length pattern"));
            }
            let groups = !o.fixed && has_backrefs(srep);
            let re = Re::compile(interp, &p, o.fixed, o.perl, o.icase && !o.fixed, groups)?;
            let mut out = Vec::with_capacity(n);
            for e in &tv.data {
                let Some(s) = e else {
                    out.push(None);
                    continue;
                };
                let s: &str = s;
                let mut buf = String::with_capacity(s.len());
                let mut nmatch = 0;
                let mut offset = 0usize;
                let mut last_end: i64 = -1;
                let mut notbol = false;
                let ctx_of = |notbol: bool| if o.perl { Ctx::Full } else { Ctx::Fresh { notbol } };
                while let Some(m) = re.exec(s, offset, ctx_of(notbol), groups) {
                    let (so, eo) = m[0].unwrap();
                    nmatch += 1;
                    buf.push_str(&s[offset..so]);
                    if o.fixed {
                        buf.push_str(srep);
                    } else if eo as i64 > last_end {
                        string_adj(&mut buf, s, srep, &m, o.perl);
                        last_end = eo as i64;
                    }
                    offset = eo;
                    if offset >= s.len() || !global {
                        break;
                    }
                    if eo == so {
                        let cl = char_len_at(s, offset);
                        buf.push_str(&s[offset..offset + cl]);
                        offset += cl;
                    }
                    notbol = true;
                }
                if nmatch == 0 {
                    out.push(e.clone());
                } else if repl.is_none() {
                    out.push(None);
                } else {
                    buf.push_str(&s[offset..]);
                    out.push(Some(Rc::from(buf.as_str())));
                }
            }
            out
        }
    };
    let mut ans = Value::strs(out);
    copy_all_attrs(text, &mut ans);
    Ok(ans)
}

fn do_sub(i: &mut Interp, c: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    gsub_impl(i, c, a, false)
}

fn do_gsub(i: &mut Interp, c: &Value, a: Vec<Arg>, _: &Env) -> RResult<Value> {
    gsub_impl(i, c, a, true)
}

// ======================================================================
// regexpr, gregexpr, regexec

fn set_attr(v: &mut Value, name: &str, val: Value) {
    v.set_attr_raw(Sym::new(name), val);
}

fn set_bytes_attrs(v: &mut Value, use_bytes: bool, itype: &str) {
    if use_bytes {
        set_attr(v, "index.type", Value::str(itype));
        set_attr(v, "useBytes", Value::lgl(true));
    }
}

/// Position (1-based) and length of a byte span, in chars unless
/// `use_bytes`.
fn pos_len(s: &str, span: Option<(usize, usize)>, use_bytes: bool) -> (i32, i32) {
    match span {
        None => (0, 0),
        Some((a, b)) => {
            if use_bytes {
                (a as i32 + 1, (b - a) as i32)
            } else {
                (nchars(&s[..a]) as i32 + 1, nchars(&s[a..b]) as i32)
            }
        }
    }
}

fn int_matrix(data: Vec<i32>, nrow: usize, ncol: usize, names: &[RStr]) -> Value {
    let mut m = Value::ints(data);
    m.set_attr_raw(s::DIM, Value::ints(vec![nrow as i32, ncol as i32]));
    m.set_attr_raw(s::DIMNAMES, Value::list(vec![Value::Null, Value::strs(names.to_vec())]));
    m
}

struct RegexprSetup {
    re: Re,
    use_bytes: bool,
    itype: &'static str,
    perl: bool,
    fixed: bool,
}

fn regexpr_setup(interp: &mut Interp, call: &Value, args: &[Arg]) -> RResult<(RegexprSetup, Vector<RStr>)> {
    let pat = &args[0].value;
    let text = &args[1].value;
    let mut o = Opts {
        icase: as_bool2(&args[2].value, call)?,
        perl: as_bool2(&args[3].value, call)?,
        fixed: as_bool2(&args[4].value, call)?,
    };
    let use_bytes = as_bool2(&args[5].value, call)?;
    fix_opts(interp, &mut o)?;
    let p = match pat {
        Value::Str(p) if !p.data.is_empty() && p.data[0].is_some() => p.clone(),
        _ => return Err(Flow::error("invalid 'pattern' argument")),
    };
    if p.data.len() > 1 {
        interp.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    let Some(tv) = str_vec(text) else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let itype = if use_bytes { "bytes" } else { "chars" };
    let use_bytes = use_bytes || (only_ascii(&p.data[..1]) && only_ascii(&tv.data));
    let pstr = p.data[0].clone().unwrap();
    let re = Re::compile(interp, &pstr, o.fixed, o.perl, o.icase && !o.fixed, o.perl)?;
    Ok((RegexprSetup { re, use_bytes, itype, perl: o.perl, fixed: o.fixed }, tv.clone()))
}

// R: grep.c:do_regexpr (regexpr)
fn do_regexpr(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 6, "regexpr")?;
    let (st, tv) = regexpr_setup(interp, call, &args)?;
    let n = tv.data.len();
    let ncap = if st.perl { st.re.nsub } else { 0 };
    let mut pos = vec![0i32; n];
    let mut mlen = vec![0i32; n];
    let mut cs = vec![NA_INTEGER; n * ncap];
    let mut cl = vec![NA_INTEGER; n * ncap];
    for (i, e) in tv.data.iter().enumerate() {
        let Some(s) = e else {
            pos[i] = NA_INTEGER;
            mlen[i] = NA_INTEGER;
            continue;
        };
        let ctx = Ctx::Fresh { notbol: false };
        match st.re.exec(s, 0, ctx, st.perl) {
            Some(m) => {
                let (p, l) = pos_len(s, m[0], st.use_bytes);
                pos[i] = p;
                mlen[i] = l;
                for k in 0..ncap {
                    let (p, l) = pos_len(s, m.get(k + 1).copied().flatten(), st.use_bytes);
                    cs[i + k * n] = p;
                    cl[i + k * n] = l;
                }
            }
            None => {
                pos[i] = -1;
                mlen[i] = -1;
                for k in 0..ncap {
                    cs[i + k * n] = -1;
                    cl[i + k * n] = -1;
                }
            }
        }
    }
    let _ = st.fixed;
    let mut ans = Value::ints(pos);
    set_attr(&mut ans, "match.length", Value::ints(mlen));
    set_bytes_attrs(&mut ans, st.use_bytes, st.itype);
    if st.perl && ncap > 0 {
        let names = st.re.capture_names();
        set_attr(&mut ans, "capture.start", int_matrix(cs, n, ncap, &names));
        set_attr(&mut ans, "capture.length", int_matrix(cl, n, ncap, &names));
        set_attr(&mut ans, "capture.names", Value::strs(names));
    }
    Ok(ans)
}

// R: grep.c:do_regexpr (gregexpr)
fn do_gregexpr(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 6, "gregexpr")?;
    let (st, tv) = regexpr_setup(interp, call, &args)?;
    let ncap = if st.perl { st.re.nsub } else { 0 };
    let names = st.re.capture_names();
    let mut out = Vec::with_capacity(tv.data.len());
    for e in &tv.data {
        let Some(s) = e else {
            let mut v = Value::int(NA_INTEGER);
            set_attr(&mut v, "match.length", Value::int(NA_INTEGER));
            out.push(v);
            continue;
        };
        let s: &str = s;
        let mut pos = Vec::new();
        let mut mlen = Vec::new();
        let mut caps: Vec<Vec<(i32, i32)>> = Vec::new();
        let mut offset = 0usize;
        let mut notbol = false;
        loop {
            if !st.perl && !st.fixed && offset >= s.len() {
                break;
            }
            let ctx = if st.perl { Ctx::Full } else { Ctx::Fresh { notbol } };
            let Some(m) = st.re.exec(s, offset, ctx, st.perl) else { break };
            let (so, eo) = m[0].unwrap();
            let (p, l) = pos_len(s, m[0], st.use_bytes);
            pos.push(p);
            mlen.push(l);
            if ncap > 0 {
                caps.push((0..ncap).map(|k| pos_len(s, m.get(k + 1).copied().flatten(), st.use_bytes)).collect());
            }
            if eo == so {
                offset = so + if so < s.len() { char_len_at(s, so) } else { 1 };
            } else {
                offset = eo;
            }
            notbol = true;
            if offset >= s.len() {
                break;
            }
        }
        let found = !pos.is_empty();
        if !found {
            pos.push(-1);
            mlen.push(-1);
        }
        let mut v = Value::ints(pos);
        set_attr(&mut v, "match.length", Value::ints(mlen));
        set_bytes_attrs(&mut v, st.use_bytes, st.itype);
        if ncap > 0 {
            let nrow = if found { caps.len() } else { 1 };
            let mut cs = vec![-1; nrow * ncap];
            let mut cl = vec![-1; nrow * ncap];
            if found {
                for (j, row) in caps.iter().enumerate() {
                    for (k, &(p, l)) in row.iter().enumerate() {
                        cs[j + k * nrow] = p;
                        cl[j + k * nrow] = l;
                    }
                }
            }
            set_attr(&mut v, "capture.start", int_matrix(cs, nrow, ncap, &names));
            set_attr(&mut v, "capture.length", int_matrix(cl, nrow, ncap, &names));
            set_attr(&mut v, "capture.names", Value::strs(names.clone()));
        }
        out.push(v);
    }
    Ok(Value::list(out))
}

// R: grep.c:do_regexec
fn do_regexec(interp: &mut Interp, _call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 5, "regexec")?;
    let pat = &args[0].value;
    let text = &args[1].value;
    let lg = |v: &Value| {
        let l = as_logical(v);
        l != NA_LOGICAL && l != 0
    };
    let mut icase = lg(&args[2].value);
    let fixed = lg(&args[3].value);
    let use_bytes = lg(&args[4].value);
    if fixed && icase {
        interp.warning("argument 'ignore.case = TRUE' will be ignored")?;
        icase = false;
    }
    let p = match pat {
        Value::Str(p) if !p.data.is_empty() && p.data[0].is_some() => p.clone(),
        _ => return Err(Flow::error("invalid 'pattern' argument")),
    };
    if p.data.len() > 1 {
        interp.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    let Some(tv) = str_vec(text) else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let itype = if use_bytes { "bytes" } else { "chars" };
    let use_bytes = use_bytes || (only_ascii(&p.data[..1]) && only_ascii(&tv.data));
    let pstr = p.data[0].clone().unwrap();
    let t = tre_translate(&pstr, icase, false, fixed)
        .map_err(|r| Flow::error(format!("regcomp error: '{}'", r)))?;
    let prog = TreProg::new(&t, true).map_err(|e| unsupported(&pstr, &e))?;
    let nmatch = t.nsub + 1;
    let mut out = Vec::with_capacity(tv.data.len());
    for e in &tv.data {
        let Some(s) = e else {
            let mut v = Value::int(NA_INTEGER);
            set_attr(&mut v, "match.length", Value::int(NA_INTEGER));
            out.push(v);
            continue;
        };
        let mut v = match prog.exec(s, true) {
            Some(m) => {
                let mut pos = Vec::with_capacity(nmatch);
                let mut len = Vec::with_capacity(nmatch);
                for j in 0..nmatch {
                    let (p, l) = pos_len(s, m.get(j).copied().flatten(), use_bytes);
                    pos.push(p);
                    len.push(l);
                }
                let mut v = Value::ints(pos);
                set_attr(&mut v, "match.length", Value::ints(len));
                v
            }
            None => {
                let mut v = Value::int(-1);
                set_attr(&mut v, "match.length", Value::int(-1));
                v
            }
        };
        set_bytes_attrs(&mut v, use_bytes, itype);
        out.push(v);
    }
    Ok(Value::list(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tre(p: &str) -> String {
        tre_translate(p, false, false, false).map(|t| t.pattern).unwrap_or_else(|e| format!("ERR {}", e))
    }

    #[test]
    fn tre_errors() {
        assert_eq!(tre("("), "ERR Missing ')'");
        assert_eq!(tre("["), "ERR Missing ']'");
        assert_eq!(tre("a{2,1}"), "ERR Invalid contents of {}");
        assert_eq!(tre("a{1"), "ERR Missing '}'");
        assert_eq!(tre("a**"), "ERR Invalid use of repetition operators");
        assert_eq!(tre("[[:foo:]]"), "ERR Unknown character class name");
        assert_eq!(tre("\\"), "ERR Trailing backslash");
        assert_eq!(tre("(a)\\2"), "ERR Invalid back reference");
        assert_eq!(tre("[b-a]"), "ERR Invalid character range");
    }

    fn compiles(p: &str) {
        let t = tre_translate(p, false, false, false).unwrap();
        MetaRegex::new(&t.pattern).unwrap_or_else(|e| panic!("{} -> {}: {}", p, t.pattern, e));
        let t = tre_translate(p, true, true, false).unwrap();
        MetaRegex::new(&t.pattern).unwrap_or_else(|e| panic!("{} -> {}: {}", p, t.pattern, e));
    }

    #[test]
    fn tre_compiles() {
        for p in [
            "a|b", "^a$", "[[:alpha:]]+", "[^[:space:]]", "\\w+\\s\\d", "(a)(b)?", "a{2,3}", "x*?", ")", "*a",
            "[]a]", "[a-]", "\\<a\\>", "\\bfoo\\b", "\\.", "[[:punct:]]", "[[:print:][:graph:]]", "(?i)abc", "a.b",
            "\\x41\\x{263A}", "[\\d]", "(|a)", "()",
        ] {
            compiles(p);
        }
    }

    #[test]
    fn longest() {
        let t = tre_translate("a|ab", false, false, false).unwrap();
        let p = TreProg::new(&t, false).unwrap();
        assert_eq!(p.exec("abc", false).unwrap()[0], Some((0, 2)));
    }

    #[test]
    fn pcre() {
        assert!(pcre_translate("(").is_err());
        let t = pcre_translate(r"\w+\b").unwrap();
        fancy_regex::Regex::new(&t).unwrap();
        let t = pcre_translate(r"a++b").unwrap();
        assert_eq!(t, "(?>a++)b".replace("++", "+"));
        let t = pcre_translate(r"(?<y>\d{4})-(?<m>\d\d)").unwrap();
        fancy_regex::Regex::new(&t).unwrap();
    }
}
