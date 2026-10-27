//! Tokenizer following the rules of GNU R's `gram.y` lexer.

use crate::ast::{Const, ParseError, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Const(Const),
    Str(String),
    Sym(String),
    Function,
    Lambda,
    If,
    Else,
    For,
    In,
    While,
    Repeat,
    Break,
    Next,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    /// `[[`
    LBB,
    RBracket,
    Comma,
    Semi,
    Newline,
    /// `<-`, `<<-` or `:=`
    LeftAssign(&'static str),
    EqAssign,
    /// `->` or `->>`
    RightAssign(&'static str),
    Tilde,
    Question,
    Or,
    Or2,
    And,
    And2,
    Not,
    Cmp(&'static str),
    Plus,
    Minus,
    Star,
    Slash,
    /// `%op%`, including `%%`, `%/%` and `%in%`
    Special(String),
    Pipe,
    PipeBind,
    Colon,
    /// `^` or `**`
    Caret,
    Dollar,
    At,
    /// `::` or `:::`
    NsGet(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Warnings produced while lexing, e.g. for `1.5L`.
    pub warnings: Vec<String>,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '.'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '.' || c == '_'
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src, bytes: src.as_bytes(), pos: 0, warnings: Vec::new() }
    }

    #[inline]
    fn peek_char(&self) -> Option<char> {
        let b = *self.bytes.get(self.pos)?;
        if b < 0x80 { Some(b as char) } else { self.src[self.pos..].chars().next() }
    }

    fn peek_char_at(&self, offset: usize) -> Option<char> {
        // ASCII up to `offset`: index bytes directly.
        let mut i = self.pos;
        for k in 0..=offset {
            let b = *self.bytes.get(i)?;
            if b >= 0x80 {
                return self.src[self.pos..].chars().nth(offset);
            }
            if k == offset {
                return Some(b as char);
            }
            i += 1;
        }
        None
    }

    fn byte_at(&self, i: usize) -> u8 {
        *self.bytes.get(i).unwrap_or(&0)
    }

    fn err(&self, message: impl Into<String>, offset: usize, incomplete: bool) -> ParseError {
        ParseError { message: message.into(), offset, end: self.pos.max(offset + 1), incomplete }
    }

    pub fn tokenize(mut self) -> Result<(Vec<Token>, Vec<String>), ParseError> {
        let mut out = Vec::new();
        loop {
            let t = self.next_token()?;
            let eof = t.tok == Tok::Eof;
            out.push(t);
            if eof {
                break;
            }
        }
        Ok((out, self.warnings))
    }

    fn next_token(&mut self) -> Result<Token, ParseError> {
        // Skip horizontal whitespace and comments.
        loop {
            match self.byte_at(self.pos) {
                b' ' | b'\t' | b'\r' | 0x0c => self.pos += 1,
                b'#' => {
                    // To the end of the line (a newline byte never occurs
                    // inside a multi-byte character).
                    self.pos +=
                        self.bytes[self.pos..].iter().position(|&b| b == b'\n').unwrap_or(self.bytes.len() - self.pos);
                }
                0xc2 if self.byte_at(self.pos + 1) == 0xa0 => self.pos += 2, // U+00A0
                _ => break,
            }
        }
        let start = self.pos;
        let Some(c) = self.peek_char() else {
            return Ok(Token { tok: Tok::Eof, span: Span::new(start, start) });
        };
        let tok = match c {
            '\n' => {
                self.pos += 1;
                Tok::Newline
            }
            '0'..='9' => self.number()?,
            '.' if matches!(self.peek_char_at(1), Some('0'..='9')) => self.number()?,
            '"' | '\'' => {
                self.pos += 1;
                Tok::Str(self.string_body(c, start)?)
            }
            '`' => {
                self.pos += 1;
                Tok::Sym(self.string_body('`', start)?)
            }
            'r' | 'R' if matches!(self.peek_char_at(1), Some('"') | Some('\'')) && self.raw_string_ahead() => {
                Tok::Str(self.raw_string(start)?)
            }
            c if is_ident_start(c) => self.ident(),
            // The pipe placeholder; identifiers cannot start with `_`.
            '_' if !self.peek_char_at(1).is_some_and(is_ident_char) => {
                self.pos += 1;
                Tok::Sym("_".to_string())
            }
            _ => self.operator(start)?,
        };
        Ok(Token { tok, span: Span::new(start, self.pos) })
    }

    fn raw_string_ahead(&self) -> bool {
        // r"---( ... )---": after the quote, optional dashes then an opener.
        let mut i = self.pos + 2;
        while self.byte_at(i) == b'-' {
            i += 1;
        }
        matches!(self.byte_at(i), b'(' | b'[' | b'{')
    }

    fn raw_string(&mut self, start: usize) -> Result<String, ParseError> {
        let quote = self.bytes[self.pos + 1];
        self.pos += 2;
        let mut dashes = 0;
        while self.byte_at(self.pos) == b'-' {
            dashes += 1;
            self.pos += 1;
        }
        let close = match self.byte_at(self.pos) {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            _ => return Err(self.err("malformed raw string literal", start, false)),
        };
        self.pos += 1;
        let body_start = self.pos;
        loop {
            if self.pos >= self.bytes.len() {
                return Err(self.err("unexpected INCOMPLETE_STRING", start, true));
            }
            if self.bytes[self.pos] == close {
                let mut j = self.pos + 1;
                let mut n = 0;
                while n < dashes && self.byte_at(j) == b'-' {
                    n += 1;
                    j += 1;
                }
                if n == dashes && self.byte_at(j) == quote {
                    let body = self.src[body_start..self.pos].to_string();
                    self.pos = j + 1;
                    return Ok(body);
                }
            }
            self.pos += 1;
        }
    }

    fn ident(&mut self) -> Tok {
        let start = self.pos;
        loop {
            match self.byte_at(self.pos) {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' => self.pos += 1,
                b if b >= 0x80 => match self.peek_char() {
                    Some(c) if is_ident_char(c) => self.pos += c.len_utf8(),
                    _ => break,
                },
                _ => break,
            }
        }
        let s = &self.src[start..self.pos];
        // Slice patterns compile to length and byte tests (no string
        // comparisons per keyword).
        match s.as_bytes() {
            [b'N', b'U', b'L', b'L'] => Tok::Const(Const::Null),
            [b'N', b'A'] => Tok::Const(Const::Lgl(None)),
            [b'T', b'R', b'U', b'E'] => Tok::Const(Const::Lgl(Some(true))),
            [b'F', b'A', b'L', b'S', b'E'] => Tok::Const(Const::Lgl(Some(false))),
            [b'I', b'n', b'f'] => Tok::Const(Const::Dbl(f64::INFINITY)),
            [b'N', b'a', b'N'] => Tok::Const(Const::Dbl(f64::NAN)),
            [b'N', b'A', b'_', rest @ ..] => match rest {
                [b'i', b'n', b't', b'e', b'g', b'e', b'r', b'_'] => Tok::Const(Const::Int(None)),
                [b'r', b'e', b'a', b'l', b'_'] => Tok::Const(Const::DblNa),
                [b'c', b'h', b'a', b'r', b'a', b'c', b't', b'e', b'r', b'_'] => Tok::Const(Const::Str(None)),
                [b'c', b'o', b'm', b'p', b'l', b'e', b'x', b'_'] => Tok::Const(Const::CplxNa),
                _ => Tok::Sym(s.to_string()),
            },
            [b'f', b'u', b'n', b'c', b't', b'i', b'o', b'n'] => Tok::Function,
            [b'i', b'f'] => Tok::If,
            [b'e', b'l', b's', b'e'] => Tok::Else,
            [b'f', b'o', b'r'] => Tok::For,
            [b'i', b'n'] => Tok::In,
            [b'w', b'h', b'i', b'l', b'e'] => Tok::While,
            [b'r', b'e', b'p', b'e', b'a', b't'] => Tok::Repeat,
            [b'b', b'r', b'e', b'a', b'k'] => Tok::Break,
            [b'n', b'e', b'x', b't'] => Tok::Next,
            _ => Tok::Sym(s.to_string()),
        }
    }

    fn number(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;
        let b = self.bytes;
        let mut is_hex = false;
        let value: f64;
        let mut has_dot = false;
        let mut has_exp = false;
        if b[self.pos] == b'0' && matches!(self.byte_at(self.pos + 1), b'x' | b'X') {
            is_hex = true;
            self.pos += 2;
            let mut mantissa: f64 = 0.0;
            let mut ndigits = 0;
            let mut frac_digits = 0i32;
            while self.pos < b.len() {
                let ch = b[self.pos];
                if let Some(d) = (ch as char).to_digit(16) {
                    mantissa = mantissa * 16.0 + d as f64;
                    ndigits += 1;
                    if has_dot {
                        frac_digits += 1;
                    }
                    self.pos += 1;
                } else if ch == b'.' && !has_dot {
                    has_dot = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if ndigits == 0 {
                return Err(self.err("unexpected symbol", start, false));
            }
            let mut exp: i32 = 0;
            if matches!(self.byte_at(self.pos), b'p' | b'P') {
                has_exp = true;
                self.pos += 1;
                let mut sign = 1;
                if self.byte_at(self.pos) == b'-' {
                    sign = -1;
                    self.pos += 1;
                } else if self.byte_at(self.pos) == b'+' {
                    self.pos += 1;
                }
                let mut e: i32 = 0;
                while self.byte_at(self.pos).is_ascii_digit() {
                    e = e.saturating_mul(10).saturating_add((self.byte_at(self.pos) - b'0') as i32);
                    self.pos += 1;
                }
                exp = sign * e;
            }
            value = mantissa * 2f64.powi(exp - 4 * frac_digits);
        } else {
            while self.byte_at(self.pos).is_ascii_digit() {
                self.pos += 1;
            }
            if self.byte_at(self.pos) == b'.' {
                has_dot = true;
                self.pos += 1;
                while self.byte_at(self.pos).is_ascii_digit() {
                    self.pos += 1;
                }
            }
            if matches!(self.byte_at(self.pos), b'e' | b'E') {
                let save = self.pos;
                self.pos += 1;
                if matches!(self.byte_at(self.pos), b'+' | b'-') {
                    self.pos += 1;
                }
                if self.byte_at(self.pos).is_ascii_digit() {
                    has_exp = true;
                    while self.byte_at(self.pos).is_ascii_digit() {
                        self.pos += 1;
                    }
                } else {
                    // R accepts `1e` as 1 and treats a dangling sign as an
                    // operator; we stay conservative and roll back.
                    self.pos = save;
                }
            }
            let text = &self.src[start..self.pos];
            value = text.parse::<f64>().map_err(|_| self.err("malformed number", start, false))?;
        }
        let text_end = self.pos;
        match self.byte_at(self.pos) {
            b'L' => {
                self.pos += 1;
                let text = &self.src[start..text_end];
                if value.fract() == 0.0 && value.abs() <= i32::MAX as f64 {
                    if has_dot && !is_hex && !has_exp {
                        self.warnings.push(format!("integer literal {}L contains unnecessary decimal point", text));
                    }
                    Ok(Tok::Const(Const::Int(Some(value as i32))))
                } else if value.fract() != 0.0 {
                    self.warnings.push(format!("integer literal {}L contains decimal; using numeric value", text));
                    Ok(Tok::Const(Const::Dbl(value)))
                } else {
                    self.warnings.push(format!("non-integer value {}L qualified with L; using numeric value", text));
                    Ok(Tok::Const(Const::Dbl(value)))
                }
            }
            b'i' => {
                self.pos += 1;
                Ok(Tok::Const(Const::Cplx(0.0, value)))
            }
            _ => Ok(Tok::Const(Const::Dbl(value))),
        }
    }

    fn string_body(&mut self, quote: char, start: usize) -> Result<String, ParseError> {
        let mut out = String::new();
        let mut byte_escape = false;
        let mut unicode_escape = false;
        loop {
            // Copy a run of plain bytes at once (multi-byte characters are
            // copied whole: their bytes are all >= 0x80).
            let run = self.bytes[self.pos..]
                .iter()
                .position(|&b| b == b'\\' || b == quote as u8)
                .unwrap_or(self.bytes.len() - self.pos);
            if run > 0 {
                out.push_str(&self.src[self.pos..self.pos + run]);
                self.pos += run;
            }
            let Some(c) = self.peek_char() else {
                return Err(self.err("unexpected INCOMPLETE_STRING", start, true));
            };
            self.pos += c.len_utf8();
            if c == quote {
                if byte_escape && unicode_escape {
                    return Err(self.err(
                        "mixing Unicode and octal/hex escapes in a string is not allowed",
                        start,
                        false,
                    ));
                }
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let esc_start = self.pos - 1;
            let Some(e) = self.peek_char() else {
                return Err(self.err("unexpected INCOMPLETE_STRING", start, true));
            };
            self.pos += e.len_utf8();
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0'..='7' => {
                    byte_escape = true;
                    let mut v = e.to_digit(8).unwrap();
                    for _ in 0..2 {
                        match self.peek_char() {
                            Some(d @ '0'..='7') => {
                                v = v * 8 + d.to_digit(8).unwrap();
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    if v == 0 {
                        return Err(self.err("nul character not allowed", esc_start, false));
                    }
                    out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                }
                'a' => out.push('\u{7}'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '`' => out.push('`'),
                ' ' => out.push(' '),
                '\n' => out.push('\n'),
                'x' => {
                    byte_escape = true;
                    let mut v = 0u32;
                    let mut n = 0;
                    while n < 2 {
                        match self.peek_char().and_then(|d| d.to_digit(16)) {
                            Some(d) => {
                                v = v * 16 + d;
                                self.pos += 1;
                                n += 1;
                            }
                            None => break,
                        }
                    }
                    if n == 0 {
                        return Err(self.err("'\\x' used without hex digits in character string", esc_start, false));
                    }
                    if v == 0 {
                        return Err(self.err("nul character not allowed", esc_start, false));
                    }
                    out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                }
                'u' | 'U' => {
                    unicode_escape = true;
                    let max = if e == 'u' { 4 } else { 8 };
                    let braced = self.peek_char() == Some('{');
                    if braced {
                        self.pos += 1;
                    }
                    let mut v = 0u32;
                    let mut n = 0;
                    while n < max {
                        match self.peek_char().and_then(|d| d.to_digit(16)) {
                            Some(d) => {
                                v = v * 16 + d;
                                self.pos += 1;
                                n += 1;
                            }
                            None => break,
                        }
                    }
                    if braced {
                        if self.peek_char() != Some('}') {
                            return Err(self.err("invalid \\u{xxxx} sequence", esc_start, false));
                        }
                        self.pos += 1;
                    }
                    if n == 0 {
                        return Err(self.err(
                            format!("'\\{}' used without hex digits in character string", e),
                            esc_start,
                            false,
                        ));
                    }
                    match char::from_u32(v) {
                        Some(ch) if v != 0 => out.push(ch),
                        _ => return Err(self.err("invalid unicode escape", esc_start, false)),
                    }
                }
                other => {
                    return Err(self.err(
                        format!("'\\{}' is an unrecognized escape in character string", other),
                        esc_start,
                        false,
                    ));
                }
            }
        }
    }

    fn operator(&mut self, start: usize) -> Result<Tok, ParseError> {
        let (b0, b1, b2) = (self.byte_at(self.pos), self.byte_at(self.pos + 1), self.byte_at(self.pos + 2));
        // Longest match first, as R's lexer does.
        let (tok, len) = match b0 {
            b'<' => match (b1, b2) {
                (b'<', b'-') => (Tok::LeftAssign("<<-"), 3),
                (b'-', _) => (Tok::LeftAssign("<-"), 2),
                (b'=', _) => (Tok::Cmp("<="), 2),
                _ => (Tok::Cmp("<"), 1),
            },
            b'-' => match (b1, b2) {
                (b'>', b'>') => (Tok::RightAssign("->>"), 3),
                (b'>', _) => (Tok::RightAssign("->"), 2),
                _ => (Tok::Minus, 1),
            },
            b':' => match (b1, b2) {
                (b':', b':') => (Tok::NsGet(":::"), 3),
                (b':', _) => (Tok::NsGet("::"), 2),
                (b'=', _) => (Tok::LeftAssign(":="), 2),
                _ => (Tok::Colon, 1),
            },
            b'>' if b1 == b'=' => (Tok::Cmp(">="), 2),
            b'>' => (Tok::Cmp(">"), 1),
            b'=' if b1 == b'=' => (Tok::Cmp("=="), 2),
            b'=' if b1 == b'>' => (Tok::PipeBind, 2),
            b'=' => (Tok::EqAssign, 1),
            b'!' if b1 == b'=' => (Tok::Cmp("!="), 2),
            b'!' => (Tok::Not, 1),
            b'&' if b1 == b'&' => (Tok::And2, 2),
            b'&' => (Tok::And, 1),
            b'|' if b1 == b'|' => (Tok::Or2, 2),
            b'|' if b1 == b'>' => (Tok::Pipe, 2),
            b'|' => (Tok::Or, 1),
            b'*' if b1 == b'*' => (Tok::Caret, 2),
            b'*' => (Tok::Star, 1),
            b'[' if b1 == b'[' => (Tok::LBB, 2),
            b'[' => (Tok::LBracket, 1),
            b'~' => (Tok::Tilde, 1),
            b'?' => (Tok::Question, 1),
            b'+' => (Tok::Plus, 1),
            b'/' => (Tok::Slash, 1),
            b'^' => (Tok::Caret, 1),
            b'$' => (Tok::Dollar, 1),
            b'@' => (Tok::At, 1),
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b'{' => (Tok::LBrace, 1),
            b'}' => (Tok::RBrace, 1),
            b']' => (Tok::RBracket, 1),
            b',' => (Tok::Comma, 1),
            b';' => (Tok::Semi, 1),
            b'\\' => (Tok::Lambda, 1),
            b'%' => {
                let rest = &self.src[self.pos..];
                if let Some(end) = rest[1..].find(['%', '\n']) {
                    if rest.as_bytes()[1 + end] == b'%' {
                        let op = &rest[..end + 2];
                        self.pos += op.len();
                        return Ok(Tok::Special(op.to_string()));
                    }
                }
                return Err(self.err("unexpected input", start, false));
            }
            _ => return Err(self.err("unexpected input", start, false)),
        };
        self.pos += len;
        Ok(tok)
    }
}
