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
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
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
    internal("grepRaw", do_grepraw, 8),
    internal("agrep", do_agrep, 8),
    internal("agrepl", do_agrepl, 8),
    internal("pcre_config", do_pcre_config, 0),
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
const CLS_SPACE: &str =
    r"\t\n\x{B}\x{C}\r\x{20}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{2028}\x{2029}\x{205F}\x{3000}";
const CLS_BLANK: &str = r"\t\x{20}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{205F}\x{3000}";
const CLS_CNTRL: &str = r"\x{0}-\x{1F}\x{7F}-\x{9F}\x{2028}\x{2029}";
const CLS_PRINT: &str = r"[^\x{0}-\x{1F}\x{7F}-\x{9F}\x{2028}\x{2029}\p{Cn}]";
const CLS_GRAPH: &str =
    r"[^\x{0}-\x{20}\x{7F}-\x{9F}\x{1680}\x{2000}-\x{2006}\x{2008}-\x{200A}\x{2028}\x{2029}\x{205F}\x{3000}\p{Cn}]";
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

    // R: tre-parse.c:tre_parse_bound
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
        if (max >= 0 && min > max) || max > 255 {
            return Err(REG_BADBR);
        }
        // Count limits (`+n -n #n ~n`) and the cost equation
        // (`Xi+Yd+Zs<C`): TRE keeps scanning while the position advanced.
        // Spaces are only skipped here, after the repetition counts, so
        // `x{2, 3}` is malformed (the 3 starts a cost) while `x{2, }` is not.
        let mut counts_set = false;
        let mut costs_set = false;
        let mut approx = false;
        loop {
            let before = self.i;
            if !counts_set {
                let mut done = false;
                while self.i + 1 < self.re.len() && !done {
                    match self.peek() {
                        Some('+') | Some('-') | Some('#') => {
                            self.i += 1;
                            self.parse_int();
                            counts_set = true;
                        }
                        Some('~') => {
                            self.i += 1;
                            self.parse_int();
                            approx = true;
                        }
                        Some(',') | Some(' ') => self.i += 1,
                        _ => done = true,
                    }
                }
            }
            if !costs_set {
                let mut done = false;
                while self.i + 1 < self.re.len() && !done {
                    match self.peek() {
                        Some('+') | Some(' ') => self.i += 1,
                        Some('<') => {
                            self.i += 1;
                            while self.peek() == Some(' ') {
                                self.i += 1;
                            }
                            self.parse_int();
                            approx = true;
                        }
                        Some(',') => {
                            self.i += 1;
                            done = true;
                        }
                        Some(c) if c.is_ascii_digit() => {
                            self.parse_int();
                            match self.peek() {
                                Some('i') | Some('d') | Some('s') => {
                                    self.i += 1;
                                    costs_set = true;
                                }
                                _ => return Err(REG_BADBR),
                            }
                        }
                        _ => done = true,
                    }
                }
            }
            if self.i == before {
                break;
            }
        }
        // Missing }.
        if self.i >= self.re.len() {
            return Err(REG_EBRACE);
        }
        // Empty contents of {}.
        if self.i == start {
            return Err(REG_BADBR);
        }
        if self.peek() != Some('}') {
            return Err(REG_BADBR);
        }
        self.i += 1;
        let mut minimal = self.ungreedy;
        match self.peek() {
            Some('?') => {
                minimal = !self.ungreedy;
                self.i += 1;
            }
            Some('*') | Some('+') => return Err(REG_BADRPT),
            _ => {}
        }
        if approx || costs_set || counts_set {
            return Err("robigo: approximate regular expression matching is not supported");
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
        return Ok(TreOut {
            pattern: p,
            nsub: 0,
            has_backref: false,
            has_bol: false,
            has_minimal: false,
            has_lookaround: false,
        });
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
                let q: String;
                if c == '{' {
                    // PCRE2 allows white space inside a quantifier
                    // (`{2,    }`, `{ 2 , 3 }`), so parse the numbers and
                    // rebuild it without spaces.
                    let mut j = i + 1;
                    let num = |j: &mut usize| -> String {
                        while b.get(*j) == Some(&' ') {
                            *j += 1;
                        }
                        let s = *j;
                        while b.get(*j).is_some_and(|c| c.is_ascii_digit()) {
                            *j += 1;
                        }
                        b[s..*j].iter().collect()
                    };
                    let min = num(&mut j);
                    while b.get(j) == Some(&' ') {
                        j += 1;
                    }
                    let parsed = if min.is_empty() {
                        None
                    } else if b.get(j) == Some(&'}') {
                        Some(format!("{{{}}}", min))
                    } else if b.get(j) == Some(&',') {
                        j += 1;
                        let max = num(&mut j);
                        while b.get(j) == Some(&' ') {
                            j += 1;
                        }
                        (b.get(j) == Some(&'}')).then(|| {
                            if max.is_empty() { format!("{{{},}}", min) } else { format!("{{{},{}}}", min, max) }
                        })
                    } else {
                        None
                    };
                    let Some(parsed) = parsed else {
                        // not a quantifier: PCRE treats `{` literally
                        out.push_str(r"\{");
                        i += 1;
                        atom_start = Some(this_start);
                        last_was_quant = false;
                        continue;
                    };
                    q = parsed;
                    i = j + 1;
                } else {
                    q = c.to_string();
                    i += 1;
                }
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

/// A regex-automata meta regex using lazy DFAs only: R code typically
/// compiles a pattern per call and matches short strings, where building a
/// full DFA up front (the default for small patterns) dominates, notably
/// for Unicode classes such as `[[:alnum:]]`.
fn meta_regex(pattern: &str) -> Result<MetaRegex, regex_automata::meta::BuildError> {
    MetaRegex::builder().configure(MetaRegex::config().dfa(false)).build(pattern)
}

/// A TRE program: leftmost-longest matching via regex-automata, or
/// fancy-regex for patterns with back-references.
enum TreProg {
    Auto { meta: MetaRegex, dfa: Option<DFA>, cache: RefCell<Option<DfaCache>>, cap: Option<MetaRegex> },
    Fancy(fancy_regex::Regex),
}

type Spans = Vec<Option<(usize, usize)>>;

impl TreProg {
    fn new(t: &TreOut, want_caps: bool) -> Result<TreProg, String> {
        if t.has_backref || t.has_lookaround {
            return fancy_regex::Regex::new(&t.pattern).map(TreProg::Fancy).map_err(|e| e.to_string());
        }
        let meta = meta_regex(&t.pattern).map_err(|e| e.to_string())?;
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
        let cap = if want_caps && t.nsub > 0 { meta_regex(&format!(r"(?:{})\z", t.pattern)).ok() } else { None };
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

/// A POSIX ERE compiled for a boolean match, as R's `list.files` needs
/// (platform.c: tre_regcomp REG_EXTENDED, then `tre_regexec(...) == 0`).
/// Compile errors come back as the reason string.
pub struct TreBoolMatch(TreProg);

impl TreBoolMatch {
    pub fn compile(pat: &str, icase: bool) -> Result<TreBoolMatch, String> {
        let t = tre_translate(pat, icase, false, false).map_err(|e| e.to_string())?;
        Ok(TreBoolMatch(TreProg::new(&t, false)?))
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.0.exec(s, false).is_some()
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

/// (pattern, perl, ignore.case, captures wanted, ASCII-only text).
type ReKey = (String, bool, bool, bool, bool);

const RE_CACHE_SIZE: usize = 32;

thread_local! {
    /// Recently compiled patterns, most recent first. R code often matches
    /// the same pattern call after call (in loops, `lapply`), and for short
    /// strings compiling costs far more than matching.
    static RE_CACHE: RefCell<Vec<(ReKey, Rc<Re>)>> = const { RefCell::new(Vec::new()) };
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

    /// `ascii`: every string the pattern will be matched against is ASCII.
    /// Compiled patterns are cached (see [`RE_CACHE`]).
    fn compile(
        interp: &mut Interp,
        pat: &str,
        fixed: bool,
        perl: bool,
        icase: bool,
        want_caps: bool,
        ascii: bool,
    ) -> RResult<Rc<Re>> {
        if fixed {
            return Ok(Rc::new(Re { kind: Kind::Fixed(pat.to_string()), nsub: 0 }));
        }
        let key = (pat.to_string(), perl, icase, want_caps, ascii);
        let hit = RE_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let i = c.iter().position(|(k, _)| *k == key)?;
            let e = c.remove(i);
            let re = e.1.clone();
            c.insert(0, e);
            Some(re)
        });
        if let Some(re) = hit {
            return Ok(re);
        }
        let re = Rc::new(Re::compile_uncached(interp, pat, fixed, perl, icase, want_caps, ascii)?);
        RE_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.insert(0, (key, re.clone()));
            c.truncate(RE_CACHE_SIZE);
        });
        Ok(re)
    }

    fn compile_uncached(
        interp: &mut Interp,
        pat: &str,
        fixed: bool,
        perl: bool,
        icase: bool,
        want_caps: bool,
        ascii: bool,
    ) -> RResult<Re> {
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
        let restrict = |mut t: TreOut| {
            // (fancy-regex, used for these, has no class set operations)
            if ascii && !t.has_backref && !t.has_lookaround {
                t.pattern = ascii_classes(&t.pattern);
            }
            t
        };
        let t = restrict(tre_translate(pat, icase, false, false).map_err(|r| Re::tre_error(interp, pat, r))?);
        let std = TreProg::new(&t, want_caps).map_err(|e| unsupported(pat, &e))?;
        let notbol = if t.has_bol {
            let t2 = restrict(tre_translate(pat, icase, true, false).map_err(|r| Re::tre_error(interp, pat, r))?);
            Some(TreProg::new(&t2, want_caps).map_err(|e| unsupported(pat, &e))?)
        } else {
            None
        };
        Ok(Re { kind: Kind::Tre { std, notbol }, nsub: t.nsub })
    }

    /// Capture names for perl = TRUE ("" for unnamed groups).
    fn capture_names(&self) -> Vec<RStr> {
        match &self.kind {
            Kind::Pcre(re) => {
                re.capture_names().skip(1).map(|n| Some(crate::value::RString::from(n.unwrap_or("")))).collect()
            }
            _ => vec![Some(crate::value::RString::from("")); self.nsub],
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

/// Restrict the Unicode classes (`\p{..}`, `\P{..}`) of a translated
/// pattern to ASCII. Matching only ASCII strings gives the same results,
/// and classes such as `\p{Alphabetic}` (hundreds of ranges) become tiny
/// automata, which matters because R code compiles patterns per call.
fn ascii_classes(p: &str) -> String {
    let b = p.as_bytes();
    let mut out = String::with_capacity(p.len() + 16);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if matches!(b[i + 1], b'p' | b'P') && b.get(i + 2) == Some(&b'{') {
                if let Some(close) = p[i..].find('}') {
                    out.push('[');
                    out.push_str(&p[i..=i + close]);
                    out.push_str(r"&&\x00-\x7F]");
                    i += close + 1;
                    continue;
                }
            }
            // An escape pair; the escaped character may be multi-byte.
            let n = p[i + 1..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&p[i..i + 1 + n]);
            i += 1 + n;
            continue;
        }
        let c = p[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
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
    let toks: Vec<RStr> =
        if tv.data.is_empty() { vec![Some(crate::value::RString::from(""))] } else { tv.data.clone() };
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
                        Some(s) => Value::strs(
                            s.chars().map(|c| Some(crate::value::RString::from(c.to_string().as_str()))).collect(),
                        ),
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
                                v.push(Some(crate::value::RString::from(&rest[..p])));
                                rest = &rest[p + split.len()..];
                            }
                            if !rest.is_empty() {
                                v.push(Some(crate::value::RString::from(rest)));
                            }
                            Value::strs(v)
                        }
                    };
                }
            }
            Some(split) => {
                let re = Re::compile(interp, split, false, perl, false, false, only_ascii(&xv.data))?;
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
                                        v.push(Some(crate::value::RString::from(&s[bufp..so])));
                                        bufp = eo;
                                    } else {
                                        let cl = char_len_at(s, bufp);
                                        v.push(Some(crate::value::RString::from(&s[bufp..bufp + cl])));
                                        bufp += cl;
                                    }
                                    if bufp >= s.len() {
                                        break;
                                    }
                                }
                            }
                            if bufp < s.len() {
                                v.push(Some(crate::value::RString::from(&s[bufp..])));
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
    let mut o = Opts { icase: as_bool2(&args[2].value, call)?, perl: false, fixed: false };
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
    let re = Re::compile(interp, &p, o.fixed, o.perl, o.icase && !o.fixed, false, only_ascii(&tv.data))?;
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
// agrep / agrepl

/// TRE's approximate-match parameters (`regaparams_t`), after R's
/// `agrep.c:amatch_regaparams`.
///
/// TRE's "insert" consumes a text character and its "delete" drops a pattern
/// character, so R's `costs[0]`/`costs[1]` and `bounds[2]`/`bounds[1]` are
/// TRE's insertion/deletion costs and limits, not R's names for them.
struct AParams {
    cost_ins: i32,
    cost_del: i32,
    cost_subst: i32,
    max_cost: i32,
    max_ins: i32,
    max_del: i32,
    max_subst: i32,
    max_err: i32,
}

/// A bound from `.amatch_bounds`: fractions of the pattern length become
/// counts (`ceil`, with `NA` meaning unlimited).
fn amatch_bound(b: f64, scale: f64) -> i32 {
    if b.is_nan() {
        return i32::MAX;
    }
    let b = if b < 1.0 { b * scale } else { b };
    let c = b.ceil();
    if c >= i32::MAX as f64 { i32::MAX } else { c as i32 }
}

/// A state of TRE's approximate matcher at one pattern position
/// (`tre_tnfa_approx_reach_t`).
#[derive(Clone, Copy)]
struct Reach {
    pos: isize,
    cost: i64,
    ins: i32,
    del: i32,
    subst: i32,
    err: i32,
}

impl Reach {
    /// TRE marks everything empty with `pos = -2` (`tre-match-approx.c`).
    const EMPTY: Reach = Reach { pos: -2, cost: 0, ins: 0, del: 0, subst: 0, err: 0 };
}

/// Is there a match of `pat` in `text` within the bounds? A port of TRE's
/// `tre_tnfa_run_approx` for a literal pattern. Text before the match start
/// and after the match end is free, so the initial state is re-added at every
/// position, and a match succeeds as soon as the final state is reached.
///
/// TRE keeps one path per state and position, dropping the new path when its
/// cost ties the existing one (`cost0 >= ...`; the tag comparison cannot
/// break ties because agrep compiles with `REG_NOSUB`, so `tre_tag_order`
/// returns 0). That makes the result depend on the order the operations are
/// tried -- initial states, then inserts, then deletes, then exact matches and
/// substitutions -- so a plain edit-distance DP can disagree with TRE when
/// several alignments have the same cost. This follows TRE's loop order.
fn approx_match(pat: &[char], text: &[char], icase: bool, p: &AParams) -> bool {
    let fold = |c: char| if icase { simple_lower(c) } else { c };
    let patf: Vec<char> = pat.iter().map(|&c| fold(c)).collect();
    let textf: Vec<char> = text.iter().map(|&c| fold(c)).collect();
    if p.max_cost == 0 {
        // No cost may be spent: TRE dispatches to the exact matcher
        // (`// R: tre/regexec.c:tre_match_approx`).
        let n = patf.len();
        return n == 0 || textf.windows(n).any(|w| w == &patf[..]);
    }
    let n = pat.len();
    let m = text.len();
    let max_cost = p.max_cost as i64;
    let mut reach = vec![Reach::EMPTY; n + 1];
    let mut next = vec![Reach::EMPTY; n + 1];
    let mut prev_pos: isize = -1;
    let mut pos: isize = 0;
    loop {
        // Initial states: a match may start anywhere.
        if next[0].pos < pos {
            next[0] = Reach { pos, ..Reach::EMPTY };
            if n == 0 {
                return true;
            }
        }
        // Inserts: consume a text character, stay in the same state.
        for id in 0..=n {
            let e = reach[id];
            if e.pos != prev_pos {
                continue;
            }
            let cost = e.cost + p.cost_ins as i64;
            if cost > max_cost || e.ins + 1 > p.max_ins || e.err + 1 > p.max_err {
                continue;
            }
            let t = &mut next[id];
            if t.pos == pos && cost >= t.cost {
                continue;
            }
            *t = Reach { pos, cost, ins: e.ins + 1, del: e.del, subst: e.subst, err: e.err + 1 };
        }
        // Deletes: drop a pattern character, breadth-first, so a chain of
        // deletes can be found and a state may be updated again with a lower
        // cost.
        let mut queue: Vec<usize> = (0..=n).filter(|&i| next[i].pos == pos).collect();
        let mut qi = 0;
        while qi < queue.len() {
            let id = queue[qi];
            qi += 1;
            let e = next[id];
            let cost = e.cost + p.cost_del as i64;
            if cost > max_cost || e.del + 1 > p.max_del || e.err + 1 > p.max_err {
                continue;
            }
            let dest = id + 1;
            if dest > n {
                continue;
            }
            let t = &mut next[dest];
            if t.pos == pos && cost >= t.cost {
                continue;
            }
            *t = Reach { pos, cost, ins: e.ins, del: e.del + 1, subst: e.subst, err: e.err + 1 };
            if dest == n {
                return true;
            }
            queue.push(dest);
        }
        if pos >= m as isize {
            break;
        }
        prev_pos = pos;
        let c = text[pos as usize];
        pos += 1;
        std::mem::swap(&mut reach, &mut next);
        // Exact matches and substitutions consume `c`.
        for id in 0..n {
            let e = reach[id];
            if e.pos < prev_pos {
                continue;
            }
            let a = pat[id];
            // With `icase` TRE turns each letter into a union of its two
            // cases and emits one transition per case, upper first
            // (`// R: tre-parse.c` adds the opposite-case counterpoint and
            // the items stay sorted). Both transitions lead to the same
            // state and ties go to the first one, so a free substitution
            // (`costs[2] == 0`) can beat the exact match -- and, because it
            // is still a substitution, it also uses up `bounds[4]` errors.
            let (edges, nedges) = if icase && a.is_ascii_alphabetic() {
                ([a.to_ascii_uppercase(), a.to_ascii_lowercase()], 2)
            } else {
                ([fold(a), fold(a)], 1)
            };
            for &edge in &edges[..nedges] {
                let (cost, err) = if edge == c {
                    (e.cost, 0)
                } else {
                    let cost = e.cost + p.cost_subst as i64;
                    if cost > max_cost || e.subst + 1 > p.max_subst || e.err + 1 > p.max_err {
                        continue;
                    }
                    (cost, 1)
                };
                let t = &mut next[id + 1];
                if t.pos == pos && cost >= t.cost {
                    continue;
                }
                *t = Reach { pos, cost, ins: e.ins, del: e.del, subst: e.subst + err, err: e.err + err };
                if id + 1 == n {
                    return true;
                }
            }
        }
    }
    false
}

fn agrep_impl(interp: &mut Interp, args: Vec<Arg>, grepl: bool) -> RResult<Value> {
    check_arity_internal(&args, 8, if grepl { "agrepl" } else { "agrep" })?;
    let pat = match &args[0].value {
        Value::Str(p) if !p.data.is_empty() => p,
        _ => return Err(Flow::error("invalid 'pattern' argument")),
    };
    if pat.data.len() > 1 {
        interp.warning("argument 'pattern' has length > 1 and only the first element will be used")?;
    }
    let Some(x) = str_vec(&args[1].value) else {
        return Err(Flow::error("invalid 'x' argument"));
    };
    let icase = as_logical(&args[2].value) == TRUE;
    let value = as_logical(&args[3].value) == TRUE;
    let costs: [i32; 3] = match &args[4].value {
        Value::Int(v) if v.data.len() >= 3 => [v.data[0], v.data[1], v.data[2]],
        Value::Real(v) if v.data.len() >= 3 => [v.data[0] as i32, v.data[1] as i32, v.data[2] as i32],
        _ => [1, 1, 1],
    };
    let bounds: [f64; 5] = match &args[5].value {
        Value::Real(v) if v.data.len() >= 5 => [v.data[0], v.data[1], v.data[2], v.data[3], v.data[4]],
        Value::Int(v) if v.data.len() >= 5 => {
            [v.data[0] as f64, v.data[1] as f64, v.data[2] as f64, v.data[3] as f64, v.data[4] as f64]
        }
        _ => [0.1, f64::NAN, f64::NAN, f64::NAN, f64::NAN],
    };
    let use_bytes = as_logical(&args[6].value) == TRUE;
    let fixed = as_logical(&args[7].value) != FALSE;

    let n = x.data.len();
    if pat.data[0].is_none() {
        // An NA pattern makes every answer NA (R returns INTSXP before the
        // agrepl check, so agrepl gets integer NAs).
        if value {
            return Ok(Value::strs(vec![None; n]));
        }
        return Ok(Value::ints(vec![NA_INTEGER; n]));
    }
    let pats = pat.data[0].as_deref().unwrap_or("");
    let patlen = if use_bytes { pats.len() as i32 } else { pats.chars().count() as i32 };
    if patlen == 0 {
        return Err(Flow::error("'pattern' must be a non-empty character string"));
    }
    if !fixed {
        return Err(Flow::error("robigo: approximate matching of regular expressions is not implemented yet"));
    }
    let maxc = costs.iter().copied().max().unwrap_or(1);
    let p = AParams {
        cost_ins: costs[0],
        cost_del: costs[1],
        cost_subst: costs[2],
        max_cost: amatch_bound(bounds[0], patlen as f64 * maxc as f64),
        max_ins: amatch_bound(bounds[2], patlen as f64),
        max_del: amatch_bound(bounds[1], patlen as f64),
        max_subst: amatch_bound(bounds[3], patlen as f64),
        max_err: amatch_bound(bounds[4], patlen as f64),
    };
    let as_chars =
        |s: &str| -> Vec<char> { if use_bytes { s.bytes().map(|b| b as char).collect() } else { s.chars().collect() } };
    let pc = as_chars(pats);
    let ind: Vec<bool> = x
        .data
        .iter()
        .map(|e| match e {
            Some(s) => approx_match(&pc, &as_chars(s), icase, &p),
            None => false,
        })
        .collect();
    if grepl {
        return Ok(Value::lgls(ind.iter().map(|&b| b as Lgl).collect()));
    }
    let sel: Vec<usize> = (0..n).filter(|&i| ind[i]).collect();
    if value {
        let mut ans = Value::strs(sel.iter().map(|&i| x.data[i].clone()).collect());
        if let Some(Value::Str(nm)) = args[1].value.get_attr(s::NAMES) {
            ans.set_attr_raw(s::NAMES, Value::strs(sel.iter().map(|&i| nm.data[i].clone()).collect()));
        }
        Ok(ans)
    } else {
        Ok(Value::ints(sel.iter().map(|&i| i as i32 + 1).collect()))
    }
}

fn do_agrep(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    agrep_impl(interp, args, false)
}

fn do_agrepl(interp: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    agrep_impl(interp, args, true)
}

// ======================================================================
// grepRaw

// R: grep.c:fgrepraw1 -- fixed single binary search; `None` = no match.
fn fgrepraw1(pat: &[u8], text: &[u8], offset: usize) -> Option<usize> {
    let n = text.len();
    let nc = pat.len();
    if n < nc || offset > n - nc {
        return None;
    }
    (offset..=n - nc).find(|&i| &text[i..i + nc] == pat)
}

/// R's `do_grepraw` matches bytes. robigo's regex layer works on strings, so
/// bytes are mapped one-to-one to U+0000..U+00FF, which keeps every offset a
/// byte offset (`// R: grep.c:do_grepraw`).
fn bytes_to_latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

// R: grep.c:do_grepraw
fn do_grepraw(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity_internal(&args, 8, "grepRaw")?;
    let Value::Raw(pat) = &args[0].value else {
        return Err(Flow::error("invalid 'pattern' argument"));
    };
    let Value::Raw(text) = &args[1].value else {
        return Err(Flow::error("invalid 'text' argument"));
    };
    let pat = &pat.data;
    let text = &text.data;
    // R's offset is R_size_t: a negative or NA `asInteger` wraps to a huge
    // value and only the `offset > LENGTH(text)` test is reached.
    let offset = as_integer(&args[2].value) as u32 as usize;
    let igcase_opt = as_bool2(&args[3].value, call)?;
    let fixed_opt = as_bool2(&args[4].value, call)?;
    let value = as_bool2(&args[5].value, call)?;
    let all = as_bool2(&args[6].value, call)?;
    let mut invert = as_bool2(&args[7].value, call)?;
    if fixed_opt && igcase_opt {
        interp.warning("argument 'ignore.case = TRUE' will be ignored")?;
    }
    if invert && !value {
        interp.warning("argument 'invert = TRUE' will be ignored")?;
        invert = false;
    }
    if offset < 1 {
        return Err(Flow::error("invalid 'offset' argument"));
    }
    if offset > text.len() {
        return Ok(Value::ints(Vec::new()));
    }
    let offset = offset - 1;

    if fixed_opt {
        if pat.is_empty() {
            return Ok(if value {
                if all { Value::list(Vec::new()) } else { Value::raws(Vec::new()) }
            } else {
                Value::ints(Vec::new())
            });
        }
        if !all {
            let res = fgrepraw1(pat, text, offset);
            if invert {
                let Some(res) = res else {
                    return Ok(if value { Value::raws(text.clone()) } else { Value::ints(vec![1]) });
                };
                if !value {
                    return Ok(Value::ints(vec![if res == 0 { pat.len() as i32 } else { 0 } + 1]));
                }
                let mut out = Vec::with_capacity(text.len() - pat.len());
                out.extend_from_slice(&text[..res]);
                let after = res + pat.len();
                if after < text.len() {
                    out.extend_from_slice(&text[after..]);
                }
                return Ok(Value::raws(out));
            }
            let Some(res) = res else {
                return Ok(if value { Value::raws(Vec::new()) } else { Value::ints(Vec::new()) });
            };
            if !value {
                return Ok(Value::ints(vec![res as i32 + 1]));
            }
            return Ok(Value::raws(pat.clone()));
        }
        let mut matches: Vec<usize> = Vec::new();
        let mut off = offset;
        while off < text.len() {
            let Some(m) = fgrepraw1(pat, text, off) else { break };
            matches.push(m + 1);
            off = m + pat.len();
        }
        if value {
            if invert {
                if matches.is_empty() {
                    return Ok(Value::raws(text.clone()));
                }
                let mut ans = Vec::with_capacity(matches.len() + 1);
                let mut pos = 0usize;
                for &m in &matches {
                    let end = (m - 1) as usize;
                    ans.push(Value::raws(text[pos..end].to_vec()));
                    pos = end + pat.len();
                }
                ans.push(Value::raws(text[pos..].to_vec()));
                return Ok(Value::list(ans));
            }
            let ans: Vec<Value> = (0..matches.len()).map(|_| Value::raws(pat.clone())).collect();
            return Ok(Value::list(ans));
        }
        return Ok(Value::ints(matches.iter().map(|&m| m as i32).collect()));
    }

    // Non-fixed: TRE over the bytes (see `bytes_to_latin1`).
    let ptxt = bytes_to_latin1(pat);
    let ttxt = bytes_to_latin1(text);
    let ascii = ptxt.is_ascii() && ttxt.is_ascii();
    let re = Re::compile(interp, &ptxt, false, false, igcase_opt, false, ascii)?;

    if !all {
        let m = re.exec(&ttxt, offset, Ctx::Fresh { notbol: false }, false);
        let span = m.and_then(|m| m[0]);
        let Some((so, eo)) = span else {
            return Ok(if value { Value::raws(Vec::new()) } else { Value::ints(Vec::new()) });
        };
        if value {
            if so == eo {
                return Ok(if invert { Value::raws(text.clone()) } else { Value::raws(Vec::new()) });
            }
            if invert {
                let mut out = Vec::with_capacity(text.len() - (eo - so));
                out.extend_from_slice(&text[..so]);
                out.extend_from_slice(&text[eo..]);
                return Ok(Value::raws(out));
            }
            return Ok(Value::raws(text[offset + so..offset + eo].to_vec()));
        }
        return Ok(Value::ints(vec![(so + 1 + offset) as i32]));
    }

    // All matches: (start, length) with `start` relative to the whole text.
    // R restarts the search at each match end and sets REG_NOTBOL after the
    // first, so an empty match is detected exactly like `do_grepraw`.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut off = offset;
    let mut notbol = false;
    loop {
        let Some(m) = re.exec(&ttxt, off, Ctx::Fresh { notbol }, false) else { break };
        let Some((so, eo)) = m[0] else { break };
        if spans.is_empty() {
            notbol = true;
        }
        spans.push((so, eo - so));
        off += eo;
        if eo == 0 {
            let mut infinite = true;
            if spans.len() == 1 {
                match re.exec(&ttxt, off, Ctx::Fresh { notbol }, false).and_then(|m| m[0]) {
                    Some((_, e2)) if e2 == 0 => {}
                    _ => infinite = false,
                }
            }
            if infinite {
                interp.warning("pattern matches an empty string infinitely, returning first match only")?;
                break;
            }
        }
        if off >= text.len() {
            break;
        }
    }

    if value {
        let mut ans = Vec::new();
        let mut inv_start = 0usize;
        for &(s, len) in &spans {
            if invert {
                ans.push(Value::raws(text[inv_start..s].to_vec()));
                inv_start = s + len;
            } else {
                ans.push(Value::raws(text[s..s + len].to_vec()));
            }
        }
        if invert {
            ans.push(Value::raws(text[inv_start..].to_vec()));
        }
        return Ok(Value::list(ans));
    }
    Ok(Value::ints(spans.iter().map(|&(s, _)| s as i32 + 1).collect()))
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
            let re = Re::compile(interp, &p, o.fixed, o.perl, o.icase && !o.fixed, groups, only_ascii(&tv.data))?;
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
                    out.push(Some(crate::value::RString::from(buf.as_str())));
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
    re: Rc<Re>,
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
    let re = Re::compile(interp, &pstr, o.fixed, o.perl, o.icase && !o.fixed, o.perl, only_ascii(&tv.data))?;
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
    let t = tre_translate(&pstr, icase, false, fixed).map_err(|r| Flow::error(format!("regcomp error: '{}'", r)))?;
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

fn do_pcre_config(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut v = Value::lgls(vec![1, 1, 1, 0]);
    let names = Value::strs(vec![
        Some("UTF-8".into()),
        Some("Unicode properties".into()),
        Some("JIT".into()),
        Some("stack".into()),
    ]);
    v.set_attr_raw(s::NAMES, names);
    Ok(v)
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
        assert_eq!(tre("a{2, 3}"), "ERR Invalid contents of {}");
        assert_eq!(tre("a{1"), "ERR Missing '}'");
        assert_eq!(tre("a**"), "ERR Invalid use of repetition operators");
        assert_eq!(tre("[[:foo:]]"), "ERR Unknown character class name");
        assert_eq!(tre("\\"), "ERR Trailing backslash");
        assert_eq!(tre("(a)\\2"), "ERR Invalid back reference");
        assert_eq!(tre("[b-a]"), "ERR Invalid character range");
    }

    /// TRE only skips blanks and commas after the repetition counts, so a
    /// blank before `}` is fine but one before a second count is not.
    #[test]
    fn tre_bound_blanks() {
        assert_eq!(tre("a{2, }"), "(?:a){2,}");
        assert_eq!(tre("a{2,    }"), "(?:a){2,}");
        assert_eq!(tre("a{2, 3}"), "ERR Invalid contents of {}");
    }

    #[test]
    fn approx_ties() {
        let p = |max_err| AParams {
            cost_ins: 1,
            cost_del: 1,
            cost_subst: 0,
            max_cost: i32::MAX,
            max_ins: i32::MAX,
            max_del: i32::MAX,
            max_subst: i32::MAX,
            max_err,
        };
        let cs = |s: &str| s.chars().collect::<Vec<_>>();
        // Same alignment without `icase` needs one substitution ...
        assert!(approx_match(&cs("ab"), &cs("aa"), false, &p(1)));
        // ... but with `icase` the letter has two transitions (upper first)
        // and the first one is a free substitution, which ties with the
        // exact match and wins, spending an error (`bounds[4]`).
        assert!(!approx_match(&cs("ab"), &cs("aa"), true, &p(1)));
        assert!(approx_match(&cs("ab"), &cs("aa"), true, &p(2)));
        assert!(approx_match(&cs("aa"), &cs("aa"), true, &p(0)));
        // `max_cost == 0` goes through TRE's exact matcher.
        let exact = AParams { max_cost: 0, ..p(0) };
        assert!(approx_match(&cs("ab"), &cs("xabx"), false, &exact));
        assert!(!approx_match(&cs("ba"), &cs("ab"), false, &exact));
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
            "a|b",
            "^a$",
            "[[:alpha:]]+",
            "[^[:space:]]",
            "\\w+\\s\\d",
            "(a)(b)?",
            "a{2,3}",
            "x*?",
            ")",
            "*a",
            "[]a]",
            "[a-]",
            "\\<a\\>",
            "\\bfoo\\b",
            "\\.",
            "[[:punct:]]",
            "[[:print:][:graph:]]",
            "(?i)abc",
            "a.b",
            "\\x41\\x{263A}",
            "[\\d]",
            "(|a)",
            "()",
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
