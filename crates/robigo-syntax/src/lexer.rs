//! Tokenizer following the rules of GNU R's `gram.y` lexer.

use crate::ast::{Const, ParseError, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Const(Const),
    Str(Vec<u8>),
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
    /// The pipe placeholder `_`.
    Placeholder,
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
        ParseError {
            message: message.into(),
            offset,
            end: self.pos.max(offset + 1),
            incomplete,
            at_eof: false,
            with_pos_suffix: false,
            fixed_line_col: None,
        }
    }

    /// An error raised from the lexer (`raiseLexError` in gram.y): the
    /// position is the last character still consumed, and the message is
    /// reported with a `(<input>:<line>:<col>)` suffix.
    fn lex_err(&mut self, message: impl Into<String>) -> ParseError {
        let at = self.pos.saturating_sub(1);
        ParseError {
            message: message.into(),
            offset: at,
            end: self.pos.max(at + 1),
            incomplete: false,
            at_eof: false,
            with_pos_suffix: true,
            fixed_line_col: None,
        }
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
                Tok::Str(self.string_body(c, start, false)?)
            }
            '`' => {
                self.pos += 1;
                Tok::Sym(String::from_utf8_lossy(&self.string_body('`', start, true)?).into_owned())
            }
            'r' | 'R' if matches!(self.peek_char_at(1), Some('"') | Some('\'')) && self.raw_string_ahead() => {
                Tok::Str(self.raw_string(start)?)
            }
            c if is_ident_start(c) => self.ident(),
            // The pipe placeholder: always, even before identifier
            // characters (gram.y: `if (c == '_') return Placeholder(c);`).
            '_' => {
                self.pos += 1;
                Tok::Placeholder
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
        matches!(self.byte_at(i), b'(' | b'[' | b'{' | b'|')
    }

    fn raw_string(&mut self, start: usize) -> Result<Vec<u8>, ParseError> {
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
            b'|' => b'|',
            _ => {
                self.pos += 1; // the offending delimiter is consumed
                return Err(self.lex_err("malformed raw string literal"));
            }
        };
        self.pos += 1;
        let body_start = self.pos;
        loop {
            if self.pos >= self.bytes.len() {
                self.pos = start;
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
                    let body = self.bytes[body_start..self.pos].to_vec();
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

    /// Numeric literals, a port of `NumericValue` in gram.y (`// R:
    /// gram.y:NumericValue`). Malformed numbers become the `ERROR` token,
    /// reported as "unexpected input".
    fn number(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;
        let mut c = self.byte_at(self.pos);
        self.pos += 1;
        // `seendot`: 0 none, 1 before any exponent, 2 only after one.
        let mut seendot = if c == b'.' { 1 } else { 0 };
        let mut seenexp = false;
        let mut last = c;
        let mut count = 1usize;
        let mut ended_with_l = false;
        loop {
            c = self.byte_at(self.pos);
            if !(c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'x' | b'X' | b'L')) {
                break;
            }
            self.pos += 1; // the `while` condition consumes in C
            count += 1;
            if c == b'L' {
                // "must be at the end. Won't allow 1Le3."
                ended_with_l = true;
                break;
            }
            if c == b'x' || c == b'X' {
                if count > 2 || last != b'0' {
                    self.pos -= 1; // `xxungetc` in the final else
                    break;
                }
                let mut nd = 0;
                loop {
                    c = self.byte_at(self.pos);
                    self.pos += 1;
                    if !(c.is_ascii_hexdigit() || c == b'.') {
                        break;
                    }
                    if c == b'.' {
                        if seendot == 1 {
                            return Err(self.number_error(start));
                        }
                        seendot = 1;
                    }
                    nd += 1;
                }
                if nd == 0 {
                    return Err(self.number_error(start));
                }
                if c == b'p' || c == b'P' {
                    seenexp = true;
                    let d = self.byte_at(self.pos);
                    self.pos += 1;
                    if !d.is_ascii_digit() && d != b'+' && d != b'-' {
                        return Err(self.number_error(start));
                    }
                    if d == b'+' || d == b'-' {
                        let d2 = self.byte_at(self.pos);
                        self.pos += 1;
                        if !d2.is_ascii_digit() {
                            return Err(self.number_error(start));
                        }
                    }
                    // The first digit is validated and consumed above; count
                    // it, then scan the rest.
                    let mut nd = 1;
                    while self.byte_at(self.pos).is_ascii_digit() {
                        self.pos += 1;
                        nd += 1;
                    }
                    let _ = nd;
                    c = self.byte_at(self.pos);
                    self.pos += 1; // the `for` increment consumes in C
                }
                if c == b'L' {
                    ended_with_l = true;
                } else {
                    self.pos -= 1; // terminator is not part of the literal
                }
                break;
            }
            if c == b'e' || c == b'E' {
                if seenexp {
                    self.pos -= 1;
                    break;
                }
                seenexp = true;
                seendot = if seendot == 1 { seendot } else { 2 };
                let d = self.byte_at(self.pos);
                self.pos += 1;
                if !d.is_ascii_digit() && d != b'+' && d != b'-' {
                    return Err(self.number_error(start));
                }
                if d == b'+' || d == b'-' {
                    let d2 = self.byte_at(self.pos);
                    self.pos += 1;
                    if !d2.is_ascii_digit() {
                        return Err(self.number_error(start));
                    }
                }
                c = d; // the validated digit is pushed at the loop bottom
                last = c;
                continue;
            }
            if c == b'.' {
                if seendot > 0 {
                    self.pos -= 1;
                    break;
                }
                seendot = 1;
            }
            last = c;
        }
        let c = self.byte_at(self.pos);
        let mut is_complex = false;
        if c == b'i' && !ended_with_l {
            self.pos += 1;
            is_complex = true;
        }
        // `yytext` includes a trailing `L` but not `i`.
        let text_end = self.pos - if is_complex || ended_with_l { 1 } else { 0 };
        let text = &self.src[start..text_end];
        if is_complex {
            return Ok(Tok::Const(Const::Cplx(0.0, self.parse_number(text))));
        }
        if ended_with_l {
            let text_l = &self.src[start..self.pos];
            let value = self.parse_number(text);
            let as_int = value as i32;
            if value != as_int as f64 {
                if seendot == 1 && !seenexp {
                    self.warnings.push(format!("integer literal {}L contains decimal; using numeric value", text_l));
                } else {
                    self.warnings.push(format!("non-integer value {}L qualified with L; using numeric value", text_l));
                }
                return Ok(Tok::Const(Const::Dbl(value)));
            }
            if seendot == 1 && !seenexp {
                self.warnings.push(format!("integer literal {}L contains unnecessary decimal point", text_l));
            }
            return Ok(Tok::Const(Const::Int(Some(as_int))));
        }
        Ok(Tok::Const(Const::Dbl(self.parse_number(text))))
    }

    /// The `ERROR` token of gram.y's lexer: the scanned characters are the
    /// token; bison reports "unexpected input" at its start.
    fn number_error(&self, start: usize) -> ParseError {
        ParseError {
            message: "unexpected input".into(),
            offset: start,
            end: self.pos,
            incomplete: false,
            at_eof: false,
            with_pos_suffix: false,
            fixed_line_col: None,
        }
    }

    /// `R_atof`: decimal via the platform `strtod`, hexadecimal floats by
    /// hand (Rust's parser has no hex float syntax).
    fn parse_number(&self, text: &str) -> f64 {
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            let (mantissa_text, exp) = match hex.find(['p', 'P']) {
                Some(i) => (&hex[..i], hex[i + 1..].parse::<i32>().unwrap_or(0)),
                None => (hex, 0),
            };
            let (int_part, frac_part) = match mantissa_text.split_once('.') {
                Some((i, f)) => (i, f),
                None => (mantissa_text, ""),
            };
            let mut mantissa = 0f64;
            for d in int_part.chars().chain(frac_part.chars()) {
                mantissa = mantissa * 16.0 + d.to_digit(16).unwrap_or(0) as f64;
            }
            let frac_digits = frac_part.len() as i32;
            return mantissa * 2f64.powi(exp - 4 * frac_digits);
        }
        text.parse::<f64>().unwrap_or(f64::NAN)
    }

    /// String constants and backtick symbols, a port of `StringValue` in
    /// gram.y (`// R: gram.y:StringValue`). Produces raw bytes.
    fn string_body(&mut self, quote: char, start: usize, for_symbol: bool) -> Result<Vec<u8>, ParseError> {
        let mut out: Vec<u8> = Vec::new();
        let mut byte_escape = false;
        let mut unicode_escape = false;
        loop {
            // Copy a run of plain bytes at once (multi-byte characters are
            // copied whole: their bytes are all >= 0x80).
            let run = self.bytes[self.pos..]
                .iter()
                .position(|&b| b == b'\\' || b == quote as u8 || b == b'\n')
                .unwrap_or(self.bytes.len() - self.pos);
            if run > 0 {
                out.extend_from_slice(&self.bytes[self.pos..self.pos + run]);
                self.pos += run;
            }
            let Some(c) = self.peek_char() else {
                return Err(self.err("unexpected INCOMPLETE_STRING", start, true));
            };
            self.pos += c.len_utf8();
            if c == quote {
                if byte_escape && unicode_escape {
                    // Position: the closing quote, the last character read.
                    return Err(self.lex_err("mixing Unicode and octal/hex escapes in a string is not allowed"));
                }
                return Ok(out);
            }
            // A raw newline acts as an escaped newline: both are kept
            // (the Bravington fix in gram.y's StringValue).
            if c != '\\' && c != '\n' {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                continue;
            }
            if c == '\n' {
                out.push(b'\n');
                continue;
            }
            if let Some(ch) = self.read_escape(quote, start, for_symbol, &mut out, &mut byte_escape, &mut unicode_escape)? {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }

    /// One escape sequence after the backslash. `Ok(None)` means the escape
    /// pushed its own bytes (octal/hex/unicode); an error is a
    /// `raiseLexError` with the position of the last character consumed.
    #[allow(clippy::too_many_arguments)]
    fn read_escape(
        &mut self,
        quote: char,
        start: usize,
        for_symbol: bool,
        out: &mut Vec<u8>,
        byte_escape: &mut bool,
        unicode_escape: &mut bool,
    ) -> Result<Option<char>, ParseError> {
        let _ = quote;
        let Some(e) = self.peek_char() else {
            return Err(self.err("unexpected INCOMPLETE_STRING", start, true));
        };
        self.pos += e.len_utf8();
        Ok(match e {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'v' => Some('\u{b}'),
            '\\' => Some('\\'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            ' ' => Some(' '),
            '\n' => Some('\n'),
            '0'..='7' => {
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
                    return Err(self.lex_err("nul character not allowed"));
                }
                if v > 0xff {
                    return Err(self.lex_err(format!("\\{:o} exceeds maximum allowed octal value \\377", v)));
                }
                *byte_escape = true;
                out.push(v as u8);
                None
            }
            'x' => {
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
                    return Err(self.lex_err("'\\x' used without hex digits in character string"));
                }
                if v == 0 {
                    return Err(self.lex_err("nul character not allowed"));
                }
                *byte_escape = true;
                out.push(v as u8);
                None
            }
            'u' | 'U' => {
                if for_symbol {
                    let name = if e == 'u' { "u" } else { "U" };
                    // Position: the `u`/`U` itself.
                    return Err(ParseError {
                        message: format!("\\{} sequences not supported inside backticks", placeholder_seq(name)),
                        offset: self.pos - 1,
                        end: self.pos,
                        incomplete: false,
                        at_eof: false,
                        with_pos_suffix: true,
                        fixed_line_col: None,
                    });
                }
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
                if n == 0 {
                    // The character that failed to be a digit is pushed
                    // back first; the position is the escape letter.
                    return Err(self.lex_err(format!("'\\{}' used without hex digits in character string", e)));
                }
                if braced {
                    if self.peek_char() != Some('}') {
                        if e == 'u' {
                            // GNU R formats this with an uninitialized
                            // `(line %d)` argument; the value is not
                            // reproducible. Report the deterministic part.
                            return Err(self.lex_err("invalid \\u{xxxx} sequence"));
                        }
                        return Err(self.lex_err("invalid \\U{xxxxxxxx} sequence"));
                    }
                    self.pos += 1;
                }
                if v == 0 {
                    return Err(self.lex_err("nul character not allowed"));
                }
                if v > 0x10FFFF {
                    let form = if braced { "\\U{xxxxxxxx}" } else { "\\Uxxxxxxxx" };
                    return Err(self.lex_err(format!("invalid {} value {:6x}", form, v)));
                }
                *unicode_escape = true;
                let ch = char::from_u32(v).unwrap_or('\u{fffd}');
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                None
            }
            other => {
                return Err(self.lex_err(format!("'\\{}' is an unrecognized escape in character string", other)));
            }
        })
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
            b'|' if b1 == b'>' => (Tok::Pipe, 2),
            b'|' if b1 == b'|' => (Tok::Or2, 2),
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

/// `"\uxxxx"`/`"\Uxxxxxxxx"` in the backtick error message.
fn placeholder_seq(name: &str) -> &str {
    if name == "u" {
        "uxxxx"
    } else {
        "Uxxxxxxxx"
    }
}
