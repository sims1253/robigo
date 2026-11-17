//! TRE's regexp parser (tre-parse.c): a recursive descent parser for POSIX
//! basic and extended regexps and TRE's extensions, run on an explicit
//! stack. Patterns are wide characters (`tre_char_t`); reading at or past
//! the end yields 0, the terminator R's patterns always carry, which some
//! of TRE's checks read.

use std::rc::Rc;

use super::ast::*;
use super::ctype::{self, Class};
use super::{REG_EXTENDED, REG_ICASE, REG_LITERAL, REG_NEWLINE, REG_RIGHT_ASSOC, REG_UNGREEDY, Status};
use crate::Locale;

pub(super) const TRE_CHAR_MAX: i32 = i32::MAX;
pub(super) const RE_DUP_MAX: i32 = 255;

pub(super) const ASSERT_AT_BOL: i32 = 1;
pub(super) const ASSERT_AT_EOL: i32 = 2;
pub(super) const ASSERT_CHAR_CLASS: i32 = 4;
pub(super) const ASSERT_CHAR_CLASS_NEG: i32 = 8;
pub(super) const ASSERT_AT_BOW: i32 = 16;
pub(super) const ASSERT_AT_EOW: i32 = 32;
pub(super) const ASSERT_AT_WB: i32 = 64;
pub(super) const ASSERT_AT_WB_NEG: i32 = 128;
pub(super) const ASSERT_BACKREF: i32 = 256;

/// Maximum number of character classes in a negated bracket expression.
const MAX_NEG_CLASSES: usize = 64;

// R: tre-parse.c:tre_macros
const MACROS: [(u8, &str); 12] = [
    (b't', "\t"),
    (b'n', "\n"),
    (b'r', "\r"),
    (b'f', "\x0c"),
    (b'a', "\x07"),
    (b'e', "\x1b"),
    (b'w', "[[:alnum:]_]"),
    (b'W', "[^[:alnum:]_]"),
    (b's', "[[:space:]]"),
    (b'S', "[^[:space:]]"),
    (b'd', "[[:digit:]]"),
    (b'D', "[^[:digit:]]"),
];

/// `tre_parse_re_stack_symbol_t`.
const PARSE_RE: i32 = 0;
const PARSE_ATOM: i32 = 1;
const PARSE_MARK_FOR_SUBMATCH: i32 = 2;
const PARSE_BRANCH: i32 = 3;
const PARSE_PIECE: i32 = 4;
const PARSE_CATENATION: i32 = 5;
const PARSE_POST_CATENATION: i32 = 6;
const PARSE_UNION: i32 = 7;
const PARSE_POST_UNION: i32 = 8;
const PARSE_POSTFIX: i32 = 9;
const PARSE_RESTORE_CFLAGS: i32 = 10;

const PIPE: i32 = '|' as i32;
const LPAREN: i32 = '(' as i32;
const RPAREN: i32 = ')' as i32;
const LBRACE: i32 = '{' as i32;
const RBRACE: i32 = '}' as i32;
const LBRACKET: i32 = '[' as i32;
const RBRACKET: i32 = ']' as i32;
const MINUS: i32 = '-' as i32;
const STAR: i32 = '*' as i32;
const QUESTIONMARK: i32 = '?' as i32;
const PLUS: i32 = '+' as i32;
const PERIOD: i32 = '.' as i32;
const COLON: i32 = ':' as i32;
const EQUAL: i32 = '=' as i32;
const COMMA: i32 = ',' as i32;
const CARET: i32 = '^' as i32;
const DOLLAR: i32 = '$' as i32;
const BACKSLASH: i32 = '\\' as i32;
const HASH: i32 = '#' as i32;
const TILDE: i32 = '~' as i32;

/// How a step of the parse loop ends early. C's `STACK_PUSHX` sets the
/// status and leaves the loop, so the missing-')' check still runs after
/// it; a `return` skips that check.
enum Exit {
    Break(Status),
    Return(Status),
}

macro_rules! pushx {
    ($self:ident, $push:ident($v:expr)) => {
        if let Err(s) = $self.stack.$push($v) {
            return Err(Exit::Break(s));
        }
    };
}

/// `tre_parse_ctx_t`.
pub(super) struct Parser<'p, 'a> {
    pub ast: &'a mut Ast,
    pub stack: &'a mut Stack,
    pat: &'p [i32],
    re: usize,
    re_end: usize,
    pub result: Option<NodeId>,
    pub submatch_id: i32,
    pub position: i32,
    pub max_backref: i32,
    pub have_approx: bool,
    pub cflags: i32,
    nofirstsub: bool,
    /// `cur_max == 1`: classes in brackets expand to byte ranges.
    cur_max_one: bool,
    locale: Locale,
}

impl<'p, 'a> Parser<'p, 'a> {
    pub fn new(
        ast: &'a mut Ast,
        stack: &'a mut Stack,
        pat: &'p [i32],
        cflags: i32,
        cur_max_one: bool,
        locale: Locale,
    ) -> Self {
        Parser {
            ast,
            stack,
            pat,
            re: 0,
            re_end: pat.len(),
            result: None,
            submatch_id: 0,
            position: 0,
            max_backref: -1,
            have_approx: false,
            cflags,
            nofirstsub: false,
            cur_max_one,
            locale,
        }
    }

    /// The pattern character at `i`; 0 at and past the end.
    #[inline]
    fn c(&self, i: usize) -> i32 {
        self.pat.get(i).copied().unwrap_or(0)
    }

    #[inline]
    fn isctype(&self, c: i32, k: Class) -> bool {
        k.contains(c as u32, self.locale)
    }
    #[inline]
    fn islower(&self, c: i32) -> bool {
        self.isctype(c, Class::Lower)
    }
    #[inline]
    fn isupper(&self, c: i32) -> bool {
        self.isctype(c, Class::Upper)
    }
    #[inline]
    fn toupper(&self, c: i32) -> i32 {
        ctype::toupper(c as u32, self.locale) as i32
    }
    #[inline]
    fn tolower(&self, c: i32) -> i32 {
        ctype::tolower(c as u32, self.locale) as i32
    }

    fn has(&self, flag: i32) -> bool {
        self.cflags & flag != 0
    }

    fn result(&self) -> NodeId {
        self.result.expect("TRE: parse step without a result")
    }

    // R: tre-parse.c:tre_new_item
    fn new_item(&mut self, min: i32, max: i32, items: &mut Vec<NodeId>, max_i: &mut usize) -> Result<(), Status> {
        if items.len() >= *max_i {
            // TRE gives up past 1024 items: probably a missing ']'.
            if *max_i > 1024 {
                return Err(Status::Espace);
            }
            *max_i *= 2;
        }
        items.push(self.ast.new_literal(min, max, -1));
        Ok(())
    }

    // R: tre-parse.c:tre_expand_ctype
    fn expand_ctype(&mut self, class: Class, items: &mut Vec<NodeId>, max_i: &mut usize) -> Result<(), Status> {
        let icase = self.has(REG_ICASE);
        let (mut min, mut max) = (-1, 0);
        for c in 0..256 {
            if self.isctype(c, class)
                || (icase && (self.isctype(self.tolower(c), class) || self.isctype(self.toupper(c), class)))
            {
                if min < 0 {
                    min = c;
                }
                max = c;
            } else if min >= 0 {
                self.new_item(min, max, items, max_i)?;
                min = -1;
            }
        }
        if min >= 0 {
            self.new_item(min, max, items, max_i)?;
        }
        Ok(())
    }

    // R: tre-parse.c:tre_parse_bracket_items
    fn parse_bracket_items(
        &mut self,
        negate: bool,
        neg_classes: &mut Vec<Class>,
        items: &mut Vec<NodeId>,
        max_i: &mut usize,
    ) -> Result<(), Status> {
        let start = self.re;
        let mut re = self.re;
        let status = loop {
            let mut skip = false;
            if re == self.re_end {
                break Err(Status::Ebrack);
            }
            if self.c(re) == RBRACKET && re > start {
                re += 1;
                break Ok(());
            }
            let mut class = None;
            let (min, max): (i32, i32);
            if re + 2 < self.re_end && self.c(re + 1) == MINUS && self.c(re + 2) != RBRACKET {
                min = self.c(re);
                max = self.c(re + 2);
                re += 3;
                // XXX - Should use collation order instead of encoding
                // values in character ranges.
                if (min as u32) > (max as u32) {
                    break Err(Status::Erange);
                }
            } else if re + 1 < self.re_end
                && self.c(re) == LBRACKET
                && (self.c(re + 1) == PERIOD || self.c(re + 1) == EQUAL)
            {
                break Err(Status::Ecollate);
            } else if re + 1 < self.re_end && self.c(re) == LBRACKET && self.c(re + 1) == COLON {
                let mut endptr = re + 2;
                while endptr < self.re_end && self.c(endptr) != COLON {
                    endptr += 1;
                }
                if endptr == self.re_end {
                    break Err(Status::Ectype);
                }
                // At most 63 characters, up to a NUL (wcsncpy).
                let len = (endptr - re - 2).min(63);
                let name = &self.pat[re + 2..re + 2 + len];
                let name = match name.iter().position(|&c| c == 0) {
                    Some(n) => &name[..n],
                    None => name,
                };
                class = Class::from_name(name, self.locale);
                let Some(k) = class else {
                    break Err(Status::Ectype);
                };
                // Optimize character classes for 8 bit character sets.
                if self.cur_max_one {
                    if let Err(s) = self.expand_ctype(k, items, max_i) {
                        break Err(s);
                    }
                    class = None;
                    skip = true;
                }
                // The ']' after ':' is not checked.
                re = endptr + 2;
                min = 0;
                max = TRE_CHAR_MAX;
            } else {
                if self.c(re) == MINUS && self.c(re + 1) != RBRACKET && start != re {
                    // Two ranges are not allowed to share an endpoint.
                    break Err(Status::Erange);
                }
                min = self.c(re);
                max = min;
                re += 1;
            }

            if let (Some(k), true) = (class, negate) {
                if neg_classes.len() >= MAX_NEG_CLASSES {
                    break Err(Status::Espace);
                }
                neg_classes.push(k);
            } else if !skip {
                if let Err(s) = self.new_item(min, max, items, max_i) {
                    break Err(s);
                }
                let last = *items.last().unwrap();
                self.ast.lit_mut(last).class = class;
            }

            // Add opposite-case counterpoints if REG_ICASE is present.
            // This is broken if there are more than two "same" characters.
            if self.has(REG_ICASE) && class.is_none() && !skip {
                // tre_cint_t is unsigned.
                let (mut min, max) = (min as u32, max as u32);
                let mut status = Ok(());
                while min <= max {
                    let c = min as i32;
                    if self.islower(c) {
                        let cmin = self.toupper(c);
                        let mut ccurr = cmin;
                        min += 1;
                        while self.islower(min as i32)
                            && self.toupper(min as i32) == ccurr.wrapping_add(1)
                            && min <= max
                        {
                            ccurr = self.toupper(min as i32);
                            min += 1;
                        }
                        status = self.new_item(cmin, ccurr, items, max_i);
                    } else if self.isupper(c) {
                        let cmin = self.tolower(c);
                        let mut ccurr = cmin;
                        min += 1;
                        while self.isupper(min as i32)
                            && self.tolower(min as i32) == ccurr.wrapping_add(1)
                            && min <= max
                        {
                            ccurr = self.tolower(min as i32);
                            min += 1;
                        }
                        status = self.new_item(cmin, ccurr, items, max_i);
                    } else {
                        min += 1;
                    }
                    if status.is_err() {
                        break;
                    }
                }
                if let Err(s) = status {
                    break Err(s);
                }
            }
        };
        self.re = re;
        status
    }

    fn neg_classes_rc(neg_classes: &[Class]) -> Option<Rc<[Class]>> {
        if neg_classes.is_empty() { None } else { Some(Rc::from(neg_classes)) }
    }

    // R: tre-parse.c:tre_parse_bracket
    fn parse_bracket(&mut self) -> Result<NodeId, Status> {
        let mut negate = false;
        let mut items: Vec<NodeId> = Vec::new();
        let mut max_i = 32;
        let mut neg_classes = Vec::new();
        let mut node: Option<NodeId> = None;

        if self.c(self.re) == CARET {
            negate = true;
            self.re += 1;
        }

        let status = (|| {
            self.parse_bracket_items(negate, &mut neg_classes, &mut items, &mut max_i)?;
            // Sort the array if we need to negate it (glibc's qsort is a
            // stable merge sort).
            if negate {
                items.sort_by_key(|&n| self.ast.lit(n).unwrap().code_min);
            }
            let neg = Self::neg_classes_rc(&neg_classes);
            let (mut curr_max, mut curr_min) = (0i32, 0i32);
            // Build a union of the items in the array, negated if necessary.
            for &it in &items {
                let l = self.ast.lit(it).unwrap();
                let (min, max) = (l.code_min, l.code_max);
                let mut keep = true;
                if negate {
                    if min < curr_max {
                        // Overlap. TRE leaves curr_min alone here.
                        curr_max = max.wrapping_add(1).max(curr_max);
                        keep = false;
                    } else {
                        // No overlap.
                        curr_max = min.wrapping_sub(1);
                        if curr_max >= curr_min {
                            let l = self.ast.lit_mut(it);
                            l.code_min = curr_min;
                            l.code_max = curr_max;
                        } else {
                            keep = false;
                        }
                        curr_min = max.wrapping_add(1);
                        curr_max = curr_min;
                    }
                }
                if keep {
                    let position = self.position;
                    let l = self.ast.lit_mut(it);
                    l.position = position;
                    l.neg_classes = neg.clone();
                    node = Some(match node {
                        None => it,
                        Some(n) => self.ast.new_union(n, it),
                    });
                }
            }
            if negate {
                let n = self.ast.new_literal(curr_min, TRE_CHAR_MAX, self.position);
                self.ast.lit_mut(n).neg_classes = neg;
                node = Some(match node {
                    None => n,
                    Some(u) => self.ast.new_union(u, n),
                });
            }
            Ok(())
        })();
        self.position += 1;
        status.map(|()| node.expect("TRE: empty bracket expression"))
    }

    // R: tre-parse.c:tre_parse_int (with the overflow patch of TRE issue 55)
    fn parse_int(&self, r: &mut usize) -> i32 {
        let mut num: i32 = -1;
        let mut overflow = false;
        while *r < self.re_end && (b'0' as i32..=b'9' as i32).contains(&self.c(*r)) {
            if num < 0 {
                num = 0;
            }
            if num <= (i32::MAX - 9) / 10 {
                num = num * 10 + self.c(*r) - b'0' as i32;
            } else {
                overflow = true;
            }
            *r += 1;
        }
        if overflow { -1 } else { num }
    }

    /// A count limit of `tre_parse_bound`: a number, infinite if missing.
    fn parse_limit(&self, r: &mut usize) -> i32 {
        *r += 1;
        match self.parse_int(r) {
            v if v < 0 => i32::MAX,
            v => v,
        }
    }

    // R: tre-parse.c:tre_parse_bound
    fn parse_bound(&mut self) -> Result<(), Status> {
        let mut r = self.re;
        let mut minimal = self.has(REG_UNGREEDY);
        let mut approx = false;
        let mut costs_set = false;
        let mut counts_set = false;
        let (mut cost_ins, mut cost_del, mut cost_subst, mut cost_max) =
            (PARAM_UNSET, PARAM_UNSET, PARAM_UNSET, PARAM_UNSET);
        let (mut limit_ins, mut limit_del, mut limit_subst, mut limit_err) =
            (PARAM_UNSET, PARAM_UNSET, PARAM_UNSET, PARAM_UNSET);
        let is_digit = |c: i32| (b'0' as i32..=b'9' as i32).contains(&c);

        // Parse number (minimum repetition count).
        let mut min = -1;
        if r < self.re_end && is_digit(self.c(r)) {
            min = self.parse_int(&mut r);
        }
        // Parse comma and second number (maximum repetition count).
        let mut max = min;
        if r < self.re_end && self.c(r) == COMMA {
            r += 1;
            max = self.parse_int(&mut r);
        }
        // Check that the repeat counts are sane.
        if (max >= 0 && min > max) || max > RE_DUP_MAX {
            return Err(Status::Badbr);
        }

        loop {
            let start = r;
            // Parse count limit settings.
            let mut done = false;
            if !counts_set {
                while r + 1 < self.re_end && !done {
                    match self.c(r) {
                        PLUS => {
                            limit_ins = self.parse_limit(&mut r);
                            counts_set = true;
                        }
                        MINUS => {
                            limit_del = self.parse_limit(&mut r);
                            counts_set = true;
                        }
                        HASH => {
                            limit_subst = self.parse_limit(&mut r);
                            counts_set = true;
                        }
                        TILDE => {
                            limit_err = self.parse_limit(&mut r);
                            approx = true;
                        }
                        COMMA => r += 1,
                        0x20 => r += 1,
                        _ => done = true,
                    }
                }
            }

            // Parse cost restriction equation.
            done = false;
            if !costs_set {
                while r + 1 < self.re_end && !done {
                    match self.c(r) {
                        PLUS | 0x20 => r += 1,
                        0x3c => {
                            // '<'
                            r += 1;
                            while self.c(r) == 0x20 {
                                r += 1;
                            }
                            cost_max = self.parse_int(&mut r);
                            if cost_max < 0 {
                                cost_max = i32::MAX;
                            } else {
                                cost_max -= 1;
                            }
                            approx = true;
                        }
                        COMMA => {
                            r += 1;
                            done = true;
                        }
                        c if is_digit(c) => {
                            let cost = self.parse_int(&mut r);
                            // XXX - make sure r is not past end.
                            match self.c(r) {
                                0x69 => cost_ins = cost,   // 'i'
                                0x64 => cost_del = cost,   // 'd'
                                0x73 => cost_subst = cost, // 's'
                                _ => return Err(Status::Badbr),
                            }
                            r += 1;
                            costs_set = true;
                        }
                        _ => done = true,
                    }
                }
            }
            if start == r {
                break;
            }
        }

        // Missing }.
        if r >= self.re_end {
            return Err(Status::Ebrace);
        }
        // Empty contents of {}.
        if r == self.re {
            return Err(Status::Badbr);
        }
        // Parse the ending '}' or '\}'.
        if self.has(REG_EXTENDED) {
            if r >= self.re_end || self.c(r) != RBRACE {
                return Err(Status::Badbr);
            }
            r += 1;
        } else {
            if r + 1 >= self.re_end || self.c(r) != BACKSLASH || self.c(r + 1) != RBRACE {
                return Err(Status::Badbr);
            }
            r += 2;
        }

        // Parse trailing '?' marking minimal repetition.
        if r < self.re_end {
            if self.c(r) == QUESTIONMARK {
                minimal = !self.has(REG_UNGREEDY);
                r += 1;
            } else if self.c(r) == STAR || self.c(r) == PLUS {
                // These are reserved for future extensions.
                return Err(Status::Badrpt);
            }
        }

        // Create the AST node(s).
        if min == 0 && max == 0 {
            self.result = Some(self.ast.new_literal(EMPTY, -1, -1));
        } else {
            if min < 0 && max < 0 {
                // Only approximate parameters set, no repetitions.
                min = 1;
                max = 1;
            }
            let iter = self.ast.new_iter(self.result(), min, max, minimal);
            self.result = Some(iter);
            // If approximate matching parameters are set, add them to the
            // iteration node.
            if approx || costs_set || counts_set {
                if costs_set || counts_set {
                    let fill = |limit: &mut i32, cost: i32| {
                        if *limit == PARAM_UNSET {
                            *limit = if cost == PARAM_UNSET { 0 } else { i32::MAX };
                        }
                    };
                    fill(&mut limit_ins, cost_ins);
                    fill(&mut limit_del, cost_del);
                    fill(&mut limit_subst, cost_subst);
                }
                if cost_max == PARAM_UNSET {
                    cost_max = i32::MAX;
                }
                if limit_err == PARAM_UNSET {
                    limit_err = i32::MAX;
                }
                self.have_approx = true;
                let mut params = [PARAM_UNSET; PARAM_LAST];
                params[PARAM_COST_INS] = cost_ins;
                params[PARAM_COST_DEL] = cost_del;
                params[PARAM_COST_SUBST] = cost_subst;
                params[PARAM_COST_MAX] = cost_max;
                params[PARAM_MAX_INS] = limit_ins;
                params[PARAM_MAX_DEL] = limit_del;
                params[PARAM_MAX_SUBST] = limit_subst;
                params[PARAM_MAX_ERR] = limit_err;
                match &mut self.ast.nodes[iter].obj {
                    Obj::Iteration(it) => it.params = Some(Rc::new(params)),
                    _ => unreachable!(),
                }
            }
        }
        self.re = r;
        Ok(())
    }

    // R: tre-parse.c:tre_expand_macro
    fn expand_macro(&self, at: usize) -> Vec<i32> {
        if at >= self.re_end {
            return Vec::new();
        }
        let c = self.c(at);
        MACROS.iter().find(|(m, _)| *m as i32 == c).map(|(_, e)| e.bytes().map(i32::from).collect()).unwrap_or_default()
    }

    // R: tre-parse.c:tre_parse
    pub fn parse(&mut self) -> Result<(), Status> {
        let bottom = self.stack.len();
        let mut depth = 0;
        let mut temporary_cflags = 0;
        self.result = None;

        // STACK_PUSH: each push overwrites the status of the one before.
        if !self.nofirstsub {
            let _ = self.stack.push_int(self.submatch_id);
            let _ = self.stack.push_int(PARSE_MARK_FOR_SUBMATCH);
            self.submatch_id += 1;
        }
        let mut status = self.stack.push_int(PARSE_RE);

        while self.stack.len() > bottom && status.is_ok() {
            let symbol = self.stack.pop_int();
            match self.step(symbol, &mut depth, &mut temporary_cflags) {
                Ok(()) => {}
                Err(Exit::Break(s)) => status = Err(s),
                Err(Exit::Return(s)) => return Err(s),
            }
        }

        // Check for missing closing parentheses.
        if depth > 0 {
            return Err(Status::Eparen);
        }
        status
    }

    fn step(&mut self, symbol: i32, depth: &mut i32, temporary_cflags: &mut i32) -> Result<(), Exit> {
        match symbol {
            PARSE_RE => {
                // A regexp is one or more branches, separated by `|'.
                if !self.has(REG_LITERAL) && self.has(REG_EXTENDED) {
                    pushx!(self, push_int(PARSE_UNION));
                }
                pushx!(self, push_int(PARSE_BRANCH));
            }

            PARSE_BRANCH => {
                // A branch is one or more pieces, concatenated.
                pushx!(self, push_int(PARSE_CATENATION));
                pushx!(self, push_int(PARSE_PIECE));
            }

            PARSE_PIECE => {
                // An atom possibly followed by one or more postfix operators.
                if !self.has(REG_LITERAL) {
                    pushx!(self, push_int(PARSE_POSTFIX));
                }
                pushx!(self, push_int(PARSE_ATOM));
            }

            PARSE_CATENATION => {
                // If the expression has not ended, parse another piece.
                if self.re >= self.re_end {
                    return Ok(());
                }
                let c = self.c(self.re);
                let ext = self.has(REG_EXTENDED);
                if !self.has(REG_LITERAL) {
                    if ext && c == PIPE {
                        return Ok(());
                    }
                    if (ext && c == RPAREN && *depth > 0) || (!ext && c == BACKSLASH && self.c(self.re + 1) == RPAREN) {
                        let mut status = Ok(());
                        if !ext && *depth == 0 {
                            status = Err(Exit::Break(Status::Eparen));
                        }
                        *depth -= 1;
                        if !ext {
                            self.re += 2;
                        }
                        return status;
                    }
                }
                let result = self.result();
                if self.has(REG_RIGHT_ASSOC) {
                    // Right associative concatenation.
                    pushx!(self, push_node(result));
                    pushx!(self, push_int(PARSE_POST_CATENATION));
                    pushx!(self, push_int(PARSE_CATENATION));
                    pushx!(self, push_int(PARSE_PIECE));
                } else {
                    // Default case, left associative concatenation.
                    pushx!(self, push_int(PARSE_CATENATION));
                    pushx!(self, push_node(result));
                    pushx!(self, push_int(PARSE_POST_CATENATION));
                    pushx!(self, push_int(PARSE_PIECE));
                }
            }

            PARSE_POST_CATENATION => {
                let tree = self.stack.pop_node();
                self.result = Some(self.ast.new_catenation(tree, self.result()));
            }

            PARSE_UNION => {
                if self.re >= self.re_end || self.has(REG_LITERAL) {
                    return Ok(());
                }
                match self.c(self.re) {
                    PIPE => {
                        let result = self.result();
                        pushx!(self, push_int(PARSE_UNION));
                        pushx!(self, push_node(result));
                        pushx!(self, push_int(PARSE_POST_UNION));
                        pushx!(self, push_int(PARSE_BRANCH));
                        self.re += 1;
                    }
                    RPAREN => self.re += 1,
                    _ => {}
                }
            }

            PARSE_POST_UNION => {
                let tree = self.stack.pop_node();
                self.result = Some(self.ast.new_union(tree, self.result()));
            }

            PARSE_POSTFIX => self.postfix()?,

            PARSE_ATOM => self.atom(depth, temporary_cflags)?,

            PARSE_MARK_FOR_SUBMATCH => {
                let submatch_id = self.stack.pop_int();
                if let Some(mut result) = self.result {
                    if self.ast.nodes[result].submatch_id >= 0 {
                        let n = self.ast.new_literal(EMPTY, -1, -1);
                        let tmp = self.ast.new_catenation(n, result);
                        self.ast.nodes[tmp].num_submatches = self.ast.nodes[result].num_submatches;
                        result = tmp;
                    }
                    let node = &mut self.ast.nodes[result];
                    node.submatch_id = submatch_id;
                    node.num_submatches += 1;
                    self.result = Some(result);
                }
            }

            PARSE_RESTORE_CFLAGS => self.cflags = self.stack.pop_int(),

            _ => unreachable!("TRE: bad parse symbol {symbol}"),
        }
        Ok(())
    }

    fn postfix(&mut self) -> Result<(), Exit> {
        if self.re >= self.re_end || self.has(REG_LITERAL) {
            return Ok(());
        }
        let ext = self.has(REG_EXTENDED);
        let c = self.c(self.re);
        match c {
            PLUS | QUESTIONMARK | STAR if c == STAR || ext => {
                let mut minimal = self.has(REG_UNGREEDY);
                let rep_min = if c == PLUS { 1 } else { 0 };
                let rep_max = if c == QUESTIONMARK { 1 } else { -1 };
                if self.re + 1 < self.re_end {
                    let next = self.c(self.re + 1);
                    if next == QUESTIONMARK {
                        minimal = !self.has(REG_UNGREEDY);
                        self.re += 1;
                    } else if next == STAR || next == PLUS {
                        // These are reserved for future extensions.
                        return Err(Exit::Return(Status::Badrpt));
                    }
                }
                self.re += 1;
                self.result = Some(self.ast.new_iter(self.result(), rep_min, rep_max, minimal));
                pushx!(self, push_int(PARSE_POSTFIX));
            }
            // "\{" is special without REG_EXTENDED, "{" with it.
            BACKSLASH | LBRACE
                if (c == BACKSLASH && !ext && self.re + 1 < self.re_end && self.c(self.re + 1) == LBRACE)
                    || (c == LBRACE && ext) =>
            {
                if c == BACKSLASH {
                    self.re += 1;
                }
                // parse_brace:
                self.re += 1;
                self.parse_bound().map_err(Exit::Return)?;
                pushx!(self, push_int(PARSE_POSTFIX));
            }
            _ => {}
        }
        Ok(())
    }

    fn atom(&mut self, depth: &mut i32, temporary_cflags: &mut i32) -> Result<(), Exit> {
        let ret = Exit::Return;
        // End of regexp? (empty string).
        if self.re >= self.re_end || self.has(REG_LITERAL) {
            return self.parse_literal(temporary_cflags);
        }
        let ext = self.has(REG_EXTENDED);
        let re = self.re;
        let prev_backslash = re > 0 && self.c(re - 1) == BACKSLASH;
        match self.c(re) {
            LPAREN => {
                // Handle "(?...)" extensions, like Perl's.
                if ext && self.c(re + 1) == QUESTIONMARK {
                    let mut new_cflags = self.cflags;
                    let mut bit = true;
                    self.re += 2;
                    loop {
                        let flag = match self.c(self.re) {
                            0x69 => REG_ICASE,       // 'i'
                            0x6e => REG_NEWLINE,     // 'n'
                            0x72 => REG_RIGHT_ASSOC, // 'r'
                            0x55 => REG_UNGREEDY,    // 'U'
                            MINUS => {
                                self.re += 1;
                                bit = false;
                                continue;
                            }
                            COLON => {
                                self.re += 1;
                                *depth += 1;
                                break;
                            }
                            HASH => {
                                // A comment can contain any character
                                // except a right parenthesis.
                                while self.c(self.re) != RPAREN && self.re < self.re_end {
                                    self.re += 1;
                                }
                                if self.c(self.re) == RPAREN && self.re < self.re_end {
                                    self.re += 1;
                                    break;
                                }
                                return Err(ret(Status::Badpat));
                            }
                            RPAREN => {
                                self.re += 1;
                                break;
                            }
                            _ => return Err(ret(Status::Badpat)),
                        };
                        if bit {
                            new_cflags |= flag;
                        } else {
                            new_cflags &= !flag;
                        }
                        self.re += 1;
                    }
                    // Turn on the cflags changes for the rest of the
                    // enclosing group.
                    pushx!(self, push_int(self.cflags));
                    pushx!(self, push_int(PARSE_RESTORE_CFLAGS));
                    pushx!(self, push_int(PARSE_RE));
                    self.cflags = new_cflags;
                    return Ok(());
                }
                if ext || prev_backslash {
                    *depth += 1;
                    if re + 2 < self.re_end && self.c(re + 1) == QUESTIONMARK && self.c(re + 2) == COLON {
                        // Don't mark for submatching.
                        self.re += 3;
                        pushx!(self, push_int(PARSE_RE));
                    } else {
                        self.re += 1;
                        // First parse a whole RE, then mark the resulting
                        // tree for submatching.
                        pushx!(self, push_int(self.submatch_id));
                        pushx!(self, push_int(PARSE_MARK_FOR_SUBMATCH));
                        pushx!(self, push_int(PARSE_RE));
                        self.submatch_id += 1;
                    }
                    return Ok(());
                }
                self.parse_literal(temporary_cflags)
            }

            RPAREN => {
                // End of current subexpression.
                if (ext && *depth > 0) || (!ext && prev_backslash) {
                    // An atom was expected but the subexpression closed:
                    // an empty expression.
                    self.result = Some(self.ast.new_literal(EMPTY, -1, -1));
                    if !ext {
                        self.re -= 1;
                    }
                    return Ok(());
                }
                self.parse_literal(temporary_cflags)
            }

            LBRACKET => {
                self.re += 1;
                let node = self.parse_bracket().map_err(ret)?;
                self.result = Some(node);
                Ok(())
            }

            BACKSLASH => self.backslash(temporary_cflags),

            PERIOD => {
                // The any-symbol.
                if self.has(REG_NEWLINE) {
                    let t1 = self.ast.new_literal(0, '\n' as i32 - 1, self.position);
                    let t2 = self.ast.new_literal('\n' as i32 + 1, TRE_CHAR_MAX, self.position + 1);
                    self.result = Some(self.ast.new_union(t1, t2));
                    self.position += 2;
                } else {
                    self.result = Some(self.ast.new_literal(0, TRE_CHAR_MAX, self.position));
                    self.position += 1;
                }
                self.re += 1;
                Ok(())
            }

            CARET => {
                // '^' is special everywhere in EREs, and at the start of the
                // RE and after \( in BREs.
                if ext || (re >= 2 && self.c(re - 2) == BACKSLASH && self.c(re - 1) == LPAREN) || re == 0 {
                    self.result = Some(self.ast.new_literal(ASSERTION, ASSERT_AT_BOL, -1));
                    self.re += 1;
                    return Ok(());
                }
                self.parse_literal(temporary_cflags)
            }

            DOLLAR => {
                // '$' is special everywhere in EREs, and at the end of the
                // string and before \) in BREs.
                if ext
                    || (re + 2 < self.re_end && self.c(re + 1) == BACKSLASH && self.c(re + 2) == RPAREN)
                    || re + 1 == self.re_end
                {
                    self.result = Some(self.ast.new_literal(ASSERTION, ASSERT_AT_EOL, -1));
                    self.re += 1;
                    return Ok(());
                }
                self.parse_literal(temporary_cflags)
            }

            _ => self.parse_literal(temporary_cflags),
        }
    }

    fn backslash(&mut self, temporary_cflags: &mut i32) -> Result<(), Exit> {
        let ret = Exit::Return;
        let ext = self.has(REG_EXTENDED);
        // If this is "\(" or "\)" chew off the backslash and try again.
        if !ext && self.re + 1 < self.re_end && (self.c(self.re + 1) == LPAREN || self.c(self.re + 1) == RPAREN) {
            self.re += 1;
            pushx!(self, push_int(PARSE_ATOM));
            return Ok(());
        }

        // If a macro is used, parse the expanded macro recursively.
        let buf = self.expand_macro(self.re + 1);
        if !buf.is_empty() {
            let mut sub = Parser {
                ast: &mut *self.ast,
                stack: &mut *self.stack,
                pat: &buf,
                re: 0,
                re_end: buf.len(),
                result: None,
                submatch_id: self.submatch_id,
                position: self.position,
                max_backref: self.max_backref,
                have_approx: self.have_approx,
                cflags: self.cflags,
                nofirstsub: true,
                cur_max_one: self.cur_max_one,
                locale: self.locale,
            };
            sub.parse().map_err(ret)?;
            let (position, result) = (sub.position, sub.result);
            self.re += 2;
            self.position = position;
            self.result = result;
            return Ok(());
        }

        if self.re + 1 >= self.re_end {
            // Trailing backslash.
            return Err(ret(Status::Eescape));
        }

        if self.c(self.re + 1) == 'Q' as i32 {
            self.cflags |= REG_LITERAL;
            *temporary_cflags |= REG_LITERAL;
            self.re += 2;
            pushx!(self, push_int(PARSE_ATOM));
            return Ok(());
        }

        self.re += 1;
        let assertion = match self.c(self.re) {
            0x62 => Some(ASSERT_AT_WB),     // 'b'
            0x42 => Some(ASSERT_AT_WB_NEG), // 'B'
            0x3c => Some(ASSERT_AT_BOW),    // '<'
            0x3e => Some(ASSERT_AT_EOW),    // '>'
            _ => None,
        };
        if let Some(a) = assertion {
            self.result = Some(self.ast.new_literal(ASSERTION, a, -1));
            self.re += 1;
            return Ok(());
        }
        if self.c(self.re) == 'x' as i32 {
            self.re += 1;
            let xdigit = |p: &Self| p.isctype(p.c(p.re), Class::Xdigit) && p.re < p.re_end;
            if self.c(self.re) != LBRACE && self.re < self.re_end {
                // 8 bit hex char.
                let mut val: i64 = 0;
                for _ in 0..2 {
                    if xdigit(self) {
                        val = val * 16 + hex_value(self.c(self.re));
                        self.re += 1;
                    }
                }
                self.result = Some(self.ast.new_literal(val as i32, val as i32, self.position));
                self.position += 1;
                return Ok(());
            } else if self.re < self.re_end {
                // Wide char: strtol saturates at LONG_MAX, then (int).
                let mut val: i64 = 0;
                self.re += 1;
                while self.re <= self.re_end {
                    let c = self.c(self.re);
                    if c == RBRACE {
                        break;
                    }
                    if self.isctype(c, Class::Xdigit) {
                        val = val.saturating_mul(16).saturating_add(hex_value(c));
                        self.re += 1;
                        continue;
                    }
                    return Err(ret(Status::Ebrace));
                }
                self.re += 1;
                self.result = Some(self.ast.new_literal(val as i32, val as i32, self.position));
                self.position += 1;
                return Ok(());
            }
            // "\x" at the end falls through to an escaped character.
        }
        let c = self.c(self.re);
        if self.isctype(c, Class::Digit) {
            // Back reference.
            let val = c - '0' as i32;
            self.result = Some(self.ast.new_literal(BACKREF, val, self.position));
            self.position += 1;
            self.max_backref = self.max_backref.max(val);
        } else {
            // Escaped character.
            self.result = Some(self.ast.new_literal(c, c, self.position));
            self.position += 1;
        }
        self.re += 1;
        Ok(())
    }

    // R: tre-parse.c:tre_parse (parse_literal)
    fn parse_literal(&mut self, temporary_cflags: &mut i32) -> Result<(), Exit> {
        let re = self.re;
        if *temporary_cflags != 0 && re + 1 < self.re_end && self.c(re) == BACKSLASH && self.c(re + 1) == 'E' as i32 {
            self.cflags &= !*temporary_cflags;
            *temporary_cflags = 0;
            self.re += 2;
            pushx!(self, push_int(PARSE_PIECE));
            return Ok(());
        }

        // An atom was expected; if the subexpression (or the whole regexp)
        // ends here, it is an empty expression.
        let ext = self.has(REG_EXTENDED);
        let c = self.c(re);
        if !self.has(REG_LITERAL)
            && (re >= self.re_end
                || c == STAR
                || (ext && (c == PIPE || c == LBRACE || c == PLUS || c == QUESTIONMARK))
                // TRE's comment says "\)", the code tests "\{".
                || (!ext && re + 1 < self.re_end && c == BACKSLASH && self.c(re + 1) == LBRACE))
        {
            self.result = Some(self.ast.new_literal(EMPTY, -1, -1));
            return Ok(());
        }

        // R change: a literal NUL (the end of a \Q...) is empty.
        if self.has(REG_LITERAL) && c == 0 {
            self.result = Some(self.ast.new_literal(EMPTY, -1, -1));
            return Ok(());
        }

        // Not tre_isalpha(): some alphabetic characters are neither upper
        // nor lower case.
        if self.has(REG_ICASE) && (self.isupper(c) || self.islower(c)) {
            let t1 = self.ast.new_literal(self.toupper(c), self.toupper(c), self.position);
            let t2 = self.ast.new_literal(self.tolower(c), self.tolower(c), self.position);
            self.result = Some(self.ast.new_union(t1, t2));
        } else {
            self.result = Some(self.ast.new_literal(c, c, self.position));
        }
        self.position += 1;
        self.re += 1;
        Ok(())
    }
}

fn hex_value(c: i32) -> i64 {
    match c as u8 {
        b'0'..=b'9' => (c - '0' as i32) as i64,
        b'a'..=b'f' => (c - 'a' as i32 + 10) as i64,
        _ => (c - 'A' as i32 + 10) as i64,
    }
}
