//! Tokenizer following the rules of GNU R's `gram.y` lexer.

use crate::ast::{Const, ParseError, Span};

/// A symbol name: a source slice, or an owned string when escapes made
/// it differ from the source (backtick names only).
#[derive(Clone, Debug, PartialEq)]
pub enum SymStr<'a> {
    Src(&'a str),
    Own(String),
}

impl<'a> SymStr<'a> {
    /// The source slice, when the name is verbatim from the source.
    pub fn src(&self) -> Option<&'a str> {
        match self {
            SymStr::Src(s) => Some(s),
            SymStr::Own(_) => None,
        }
    }
}

impl<'a> std::fmt::Display for SymStr<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl<'a> std::ops::Deref for SymStr<'a> {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            SymStr::Src(s) => s,
            SymStr::Own(s) => s,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok<'a> {
    Const(Const),
    Str(Vec<u8>),
    /// Identifiers borrow the source: they are contiguous slices, and a
    /// borrowed token makes the parser's peeks copies instead of heap
    /// allocations (GNU R interns symbols; this is the cheap analogue).
    /// Backtick names with escapes are the exception (their bytes differ
    /// from the source), so they stay owned.
    Sym(SymStr<'a>),
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
    /// `%op%`, including `%%`, `%/%` and `%in%` (a source slice)
    Special(&'a str),
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
    /// A lexical error (`raiseLexError`/`ERROR` token in gram.y): the
    /// parser reports it when it reaches this token, as GNU R's parser
    /// does by asking the lexer for one token at a time.
    Invalid(Box<ParseError>),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token<'a> {
    pub tok: Tok<'a>,
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

    /// Tokenize the whole source. GNU R interleaves lexing with parsing:
    /// a lexical error fires only when the parser asks for that token, so
    /// a syntax error earlier in the input wins. The token stream here is
    /// eager, so the first lexical error becomes an `Invalid` token — the
    /// parser reports it when it reaches it, giving the same error
    /// (message and position) as GNU R.
    pub fn tokenize(mut self) -> Result<(Vec<Token<'a>>, Vec<String>), ParseError> {
        // R sources average ~5 bytes per token; pre-sizing avoids the
        // reallocation churn of growing one Vec per source.
        let mut out = Vec::with_capacity(self.src.len() / 4 + 16);
        loop {
            let t = match self.next_token() {
                Ok(t) => t,
                Err(e) => {
                    let span = Span { start: e.offset as u32, end: e.end.max(e.offset + 1) as u32 };
                    out.push(Token { tok: Tok::Invalid(Box::new(e)), span });
                    out.push(Token { tok: Tok::Eof, span });
                    break;
                }
            };
            let eof = t.tok == Tok::Eof;
            out.push(t);
            if eof {
                break;
            }
        }
        Ok((out, self.warnings))
    }

    fn next_token(&mut self) -> Result<Token<'a>, ParseError> {
        // Skip horizontal whitespace and comments. The loop runs on a
        // local index so the byte loads are not bounds-checked; a newline
        // byte never occurs inside a multi-byte character, so comments can
        // end at a raw byte match.
        {
            let bytes = self.bytes;
            let len = bytes.len();
            let mut i = self.pos;
            loop {
                while i < len && bytes[i] == b' ' {
                    i += 1;
                }
                if i >= len {
                    break;
                }
                match bytes[i] {
                    b'\t' | b'\r' | 0x0c => i += 1,
                    b'#' => {
                        // `position` over the raw bytes lowers to memchr.
                        i += bytes[i..].iter().position(|&b| b == b'\n').unwrap_or(len - i);
                    }
                    0xc2 if i + 1 < len && bytes[i + 1] == 0xa0 => i += 2, // U+00A0
                    _ => break,
                }
            }
            self.pos = i;
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
                let body = self.string_body('`', start, true)?;
                // Without escapes the name is the source between the
                // backticks, verbatim; with them it is the unescaped
                // bytes (e.g. `\`` is a name that is one backtick).
                let raw = &self.src[start + 1..self.pos - 1];
                let name = if raw.contains('\\') {
                    SymStr::Own(String::from_utf8_lossy(&body).into_owned())
                } else {
                    SymStr::Src(raw)
                };
                if name.is_empty() {
                    // `SymbolValue` installs the name right in the lexer
                    // (`// R: gram.y:SymbolValue`, `names.c:install`), and
                    // `install("")` errors without a position.
                    return Err(ParseError {
                        message: "attempt to use zero-length variable name".into(),
                        offset: start,
                        end: start + 2,
                        incomplete: false,
                        at_eof: false,
                        with_pos_suffix: false,
                        fixed_line_col: Some((0, 0)),
                    });
                }
                Tok::Sym(name)
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

    fn ident(&mut self) -> Tok<'a> {
        let bytes = self.bytes;
        let start = self.pos;
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            if matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_') {
                i += 1;
            } else if b >= 0x80 {
                match self.src[i..].chars().next() {
                    Some(c) if is_ident_char(c) => i += c.len_utf8(),
                    _ => break,
                }
            } else {
                break;
            }
        }
        self.pos = i;
        let s = &self.src[start..i];
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
                _ => Tok::Sym(SymStr::Src(s)),
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
            _ => Tok::Sym(SymStr::Src(s)),
        }
    }

    /// Numeric literals, a port of `NumericValue` in gram.y (`// R:
    /// gram.y:NumericValue`). Malformed numbers become the `ERROR` token,
    /// reported as "unexpected input". The scan runs on a local index over
    /// the source bytes (`byte` is 0 past the end, as `xxgetc` returns
    /// EOF); `self.pos` is synced at the exit points.
    fn number(&mut self) -> Result<Tok<'a>, ParseError> {
        let bytes = self.bytes;
        let byte = |i: usize| *bytes.get(i).unwrap_or(&0);
        let start = self.pos;
        let mut pos = self.pos;
        let mut c = byte(pos);
        pos += 1;
        // `seendot`: 0 none, 1 before any exponent, 2 only after one.
        let mut seendot = if c == b'.' { 1 } else { 0 };
        let mut seenexp = false;
        let mut last = c;
        let mut count = 1usize;
        let mut ended_with_l = false;
        loop {
            c = byte(pos);
            if !(c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'x' | b'X' | b'L')) {
                break;
            }
            pos += 1; // the `while` condition consumes in C
            count += 1;
            if c == b'L' {
                // "must be at the end. Won't allow 1Le3."
                ended_with_l = true;
                break;
            }
            if c == b'x' || c == b'X' {
                if count > 2 || last != b'0' {
                    pos -= 1; // `xxungetc` in the final else
                    break;
                }
                let mut nd = 0;
                loop {
                    c = byte(pos);
                    pos += 1;
                    if !(c.is_ascii_hexdigit() || c == b'.') {
                        break;
                    }
                    if c == b'.' {
                        if seendot == 1 {
                            self.pos = pos;
                            return Err(self.error_token(start));
                        }
                        seendot = 1;
                    }
                    nd += 1;
                }
                if nd == 0 {
                    self.pos = pos;
                    return Err(self.error_token(start));
                }
                if c == b'p' || c == b'P' {
                    seenexp = true;
                    let d = byte(pos);
                    pos += 1;
                    if !d.is_ascii_digit() && d != b'+' && d != b'-' {
                        self.pos = pos;
                        return Err(self.error_token(start));
                    }
                    if d == b'+' || d == b'-' {
                        let d2 = byte(pos);
                        pos += 1;
                        if !d2.is_ascii_digit() {
                            self.pos = pos;
                            return Err(self.error_token(start));
                        }
                    }
                    // The first digit is validated and consumed above; count
                    // it, then scan the rest.
                    let mut nd = 1;
                    while byte(pos).is_ascii_digit() {
                        pos += 1;
                        nd += 1;
                    }
                    let _ = nd;
                    c = byte(pos);
                    pos += 1; // the `for` increment consumes in C
                }
                if c == b'L' {
                    ended_with_l = true;
                } else {
                    pos -= 1; // terminator is not part of the literal
                }
                break;
            }
            if c == b'e' || c == b'E' {
                if seenexp {
                    pos -= 1;
                    break;
                }
                seenexp = true;
                seendot = if seendot == 1 { seendot } else { 2 };
                let d = byte(pos);
                pos += 1;
                if !d.is_ascii_digit() && d != b'+' && d != b'-' {
                    self.pos = pos;
                    return Err(self.error_token(start));
                }
                if d == b'+' || d == b'-' {
                    let d2 = byte(pos);
                    pos += 1;
                    if !d2.is_ascii_digit() {
                        self.pos = pos;
                        return Err(self.error_token(start));
                    }
                }
                c = d; // the validated digit is pushed at the loop bottom
                last = c;
                continue;
            }
            if c == b'.' {
                if seendot > 0 {
                    pos -= 1;
                    break;
                }
                seendot = 1;
            }
            last = c;
        }
        let c = byte(pos);
        let mut is_complex = false;
        if c == b'i' && !ended_with_l {
            pos += 1;
            is_complex = true;
        }
        self.pos = pos;
        // `yytext` includes a trailing `L` but not `i`.
        let text_end = pos - if is_complex || ended_with_l { 1 } else { 0 };
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
                    self.warnings.push(format!("integer literal {} contains decimal; using numeric value", text_l));
                } else {
                    self.warnings.push(format!("non-integer value {} qualified with L; using numeric value", text_l));
                }
                return Ok(Tok::Const(Const::Dbl(value)));
            }
            if seendot == 1 && !seenexp {
                self.warnings.push(format!("integer literal {} contains unnecessary decimal point", text_l));
            }
            return Ok(Tok::Const(Const::Int(Some(as_int))));
        }
        Ok(Tok::Const(Const::Dbl(self.parse_number(text))))
    }

    /// The `ERROR` token of gram.y's lexer, as `yyerror` reports it:
    /// "unexpected input" at the token's start (used for malformed
    /// numbers and for `<<` without the `-`).
    fn error_token(&self, start: usize) -> ParseError {
        ParseError {
            message: "unexpected input".into(),
            offset: start,
            end: self.pos.max(start + 1),
            incomplete: false,
            at_eof: false,
            with_pos_suffix: false,
            fixed_line_col: None,
        }
    }

    /// `R_atof` = `R_strtod5(str, NULL, '.', FALSE, FALSE)`: the decimal
    /// case is the x87-exact port below, the hexadecimal case is
    /// `hex_to_f64`.
    fn parse_number(&self, text: &str) -> f64 {
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return hex_to_f64(hex);
        }
        decimal_to_f64(text)
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
            if let Some(ch) =
                self.read_escape(quote, start, for_symbol, &mut out, &mut byte_escape, &mut unicode_escape)?
            {
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

    fn operator(&mut self, start: usize) -> Result<Tok<'a>, ParseError> {
        let (b0, b1, b2) = (self.byte_at(self.pos), self.byte_at(self.pos + 1), self.byte_at(self.pos + 2));
        // Longest match first, as R's lexer does.
        let (tok, len) = match b0 {
            b'<' => match (b1, b2) {
                (b'<', b'-') => (Tok::LeftAssign("<<-"), 3),
                // `<<` without the `-` is the lexer's ERROR token
                // (`// R: gram.y:yylex`, case '<'): bison then reports
                // "unexpected input" at its start.
                (b'<', _) => return Err(self.error_token(start)),
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
                        return Ok(Tok::Special(op));
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
    if name == "u" { "uxxxx" } else { "Uxxxxxxxx" }
}

/// The hexadecimal case of `R_strtod5` (`// R: util.c:R_strtod5`). `hex` is
/// the literal after `0x`/`0X`, digits with at most one `.`, then an
/// optional `p`/`P` exponent (already validated by the scanner: at least
/// one hex digit, and the exponent, if present, has at least one digit
/// after an optional sign). The C code accumulates the mantissa in an
/// x87 `LDOUBLE` and scales by powers of two; the port keeps a 64-bit
/// significand that rounds to nearest even exactly where the 80-bit
/// accumulator would, does the scaling as exponent arithmetic (a power of
/// two never rounds in either format over the grammar's exponent range),
/// and rounds to `f64` once, as the C conversion does.
fn hex_to_f64(hex: &str) -> f64 {
    let (mantissa_text, exp_text) = match hex.find(['p', 'P']) {
        Some(i) => (&hex[..i], Some(&hex[i + 1..])),
        None => (hex, None),
    };
    // The binary exponent: optional sign, digits, the accumulated value
    // capped once it passes MAX_EXPONENT_PREFIX.
    let mut expn: i32 = 0;
    if let Some(t) = exp_text {
        let (sign, digits) = match t.strip_prefix('-') {
            Some(d) => (-1, d),
            None => (1, t.strip_prefix('+').unwrap_or(t)),
        };
        let mut n: i64 = 0;
        for d in digits.bytes() {
            n = if n < 9999 { n * 10 + (d - b'0') as i64 } else { n };
        }
        expn = sign * n.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
    // Mantissa: value = m * 2^scale, m a 64-bit significand. `ans = 16*ans
    // + d` adds d at the units place of an x87 80-bit accumulator: it is
    // exact while the integer fits 64 bits; the first append past that
    // rounds to nearest even (the digit still has nibble weight then);
    // after that the accumulator's exponent is nonzero and every further
    // digit is below half an ulp, so it is dropped without a trace.
    let mut m: u64 = 0;
    let mut scale: i32 = 0;
    let mut exph: i32 = -1; // -1: no `.` yet; else 4 per fractional digit
    for c in mantissa_text.bytes() {
        if c == b'.' {
            exph = 0;
            continue;
        }
        let d = (c as char).to_digit(16).unwrap() as u128;
        let bl = 64 - m.leading_zeros() as i32;
        if scale == 0 && bl <= 60 {
            m = (m << 4) | d as u64;
        } else if scale == 0 {
            // bl(m) in 61..=64: (bl + 4) - 64 bits must go.
            let shift = (bl - 60) as u32;
            let v = ((m as u128) << 4) | d;
            let rem = v & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            let mut hi = v >> shift;
            if rem > half || (rem == half && (hi & 1) == 1) {
                hi += 1;
            }
            if hi >> 64 != 0 {
                hi >>= 1;
                scale += 1;
            }
            m = hi as u64;
            scale += shift as i32;
        } else {
            scale += 4;
        }
        if exph >= 0 {
            exph += 4;
        }
    }
    // Scaling (`if (ans != 0.0)`): `fac` is a long double built from a
    // `double p2 = 2.0` that is squared by repeated multiplication, so
    // powers beyond 2^1024 have already overflowed `p2` to infinity — a
    // division by such a `fac` underflows to 0 and a multiplication
    // overflows to Inf, whatever the true value (that is R's behavior,
    // and the spec).
    if m != 0 {
        if exph > 0 {
            if expn - exph < -122 {
                // PR#17199: divide by 2^exph now, keep expn.
                if exph >= 1024 {
                    return 0.0;
                }
                scale -= exph;
            } else {
                expn -= exph;
            }
        }
        if expn >= 1024 {
            return f64::INFINITY;
        }
        if expn <= -1024 {
            return 0.0;
        }
        scale += expn;
    }
    ld_to_f64(m, scale)
}

/// Round `m * 2^e` (m's bits the significand, as an x87 80-bit value) to
/// `f64` in one round-to-nearest-even step: 53 significant bits in the
/// normal range, the `2^-1074` grid below it, infinity past `2^1024`.
fn ld_to_f64(m: u64, e: i32) -> f64 {
    if m == 0 {
        return 0.0;
    }
    let bl = 64 - m.leading_zeros() as i32;
    let e2 = e + bl - 1; // floor(log2(value))
    if e2 >= -1022 {
        if e2 > 1024 {
            // Whole binade beyond the largest double.
            return f64::INFINITY;
        }
        let drop = bl - 53;
        let q: u64 = if drop <= 0 {
            m
        } else {
            let rem = m & ((1u64 << drop) - 1);
            let half = 1u64 << (drop - 1);
            let q = m >> drop;
            if rem > half || (rem == half && (q & 1) == 1) { q + 1 } else { q }
        };
        // value = q * 2^(e + drop) (with drop = 0 when nothing was
        // dropped); q rounds up to 2^53 at most, which overflows to
        // infinity exactly when the value would.
        (q as f64) * pow2_exact(e + drop.max(0))
    } else {
        if e2 < -1075 {
            return 0.0;
        }
        // Subnormal: round value/2^-1074 = m * 2^(e+1074) to an integer,
        // ties to even.
        // Subnormal: round value/2^-1074 = m * 2^(e+1074) to an integer,
        // ties to even. The shift can go either way (the PR#17199 path
        // can leave e far below -1074), so compute in u128.
        let s = e + 1074;
        let v = m as u128;
        let k: u64 = (if s >= 0 {
            v << s.min(64)
        } else {
            let d = (-s) as u32;
            let rem = v & ((1u128 << d) - 1);
            let half = 1u128 << (d - 1);
            let k = v >> d;
            if rem > half || (rem == half && (k & 1) == 1) { k + 1 } else { k }
        }) as u64;
        (k as f64) * f64::from_bits(1) // 2^-1074
    }
}

/// 2^k for k in [-1074, 1023], exact.
fn pow2_exact(k: i32) -> f64 {
    if k >= -1022 {
        f64::from_bits(((k + 1023) as u64) << 52)
    } else {
        f64::from_bits(1) * f64::from_bits(((k + 1074 + 1023) as u64) << 52)
    }
}

/// An x87 80-bit floating-point number, positive or zero, for
/// `R_strtod5`'s `LDOUBLE` arithmetic (`// R: util.c:R_strtod5`, the
/// decimal case). Every operation rounds to a 64-bit significand with
/// round-to-nearest-even, gradual underflow and saturating overflow, as
/// the 80-bit format does; only the final conversion to `f64` rounds to
/// 53 bits. The double rounding is observable — 17-digit literals can
/// come out 1 ulp away from a correctly-rounded parse — so exactness
/// here is what makes robigo's trees match GNU R's.
#[derive(Clone, Copy)]
struct LD {
    m: u128, // value = m * 2^e; m == 0 is zero
    e: i32,
}

impl LD {
    fn zero() -> LD {
        LD { m: 0, e: 0 }
    }
    fn small(x: u64) -> LD {
        LD { m: x as u128, e: 0 }
    }
    fn is_inf(&self) -> bool {
        self.e >= LD_INF_E
    }
}

const LD_INF_E: i32 = 1 << 24;

/// Round `m * 2^e` to the 80-bit format.
fn ld_norm(m: u128, mut e: i32) -> LD {
    if m == 0 {
        return LD::zero();
    }
    let bl = 128 - m.leading_zeros() as i32;
    let e2 = e + bl - 1;
    if e2 > 16383 {
        return LD { m: 1, e: LD_INF_E };
    }
    if e2 < -16445 {
        return LD::zero();
    }
    // 64 significand bits above the subnormal boundary, fewer below it.
    let keep = if e2 >= -16382 { 64 } else { e2 + 16446 };
    let drop = bl - keep;
    if drop <= 0 {
        return LD { m, e };
    }
    let rem = m & ((1u128 << drop) - 1);
    let half = 1u128 << (drop - 1);
    let mut q = m >> drop;
    if rem > half || (rem == half && (q & 1) == 1) {
        q += 1;
    }
    e += drop;
    if 128 - q.leading_zeros() as i32 > keep {
        q >>= 1;
        e += 1;
    }
    LD { m: q, e }
}

fn ld_mul(a: LD, b: LD) -> LD {
    if a.is_inf() || b.is_inf() {
        // 0 * inf cannot arise in R_strtod5 (the ans != 0 guard).
        return LD { m: 1, e: LD_INF_E };
    }
    if a.m == 0 || b.m == 0 {
        return LD::zero();
    }
    ld_norm(a.m * b.m, a.e + b.e)
}

/// `a / b`, correctly rounded to 64 significand bits.
fn ld_div(a: LD, b: LD) -> LD {
    if a.is_inf() {
        return a;
    }
    if b.is_inf() || a.m == 0 {
        return LD::zero();
    }
    debug_assert!(b.m != 0);
    // Normalize both mantissas into [2^63, 2^64).
    let sa = 64 - (128 - a.m.leading_zeros() as i32);
    let ma = a.m << sa;
    let ea = a.e - sa;
    let sb = 64 - (128 - b.m.leading_zeros() as i32);
    let mb = b.m << sb;
    let eb = b.e - sb;
    // quotient = (ma / mb) * 2^(ea - eb); compute 65 bits plus remainder.
    let num = ma << 64;
    let q1 = num / mb;
    let rem = num % mb;
    let (mut q, mut e) = if q1 >> 64 != 0 {
        // quotient in [1, 2): round quotient * 2^63 to an integer.
        let mut q = q1 >> 1;
        if q1 & 1 == 1 && (rem != 0 || q & 1 == 1) {
            q += 1;
        }
        (q, ea - eb - 63)
    } else {
        // quotient in [1/2, 1): round quotient * 2^64 to an integer.
        let mut q = q1;
        if 2 * rem > mb || (2 * rem == mb && q & 1 == 1) {
            q += 1;
        }
        (q, ea - eb - 64)
    };
    if 128 - q.leading_zeros() as i32 > 64 {
        q >>= 1;
        e += 1;
    }
    // q is already rounded to 64 bits; ld_norm only adds the format's
    // overflow/underflow saturation.
    ld_norm(q, e)
}

/// `a + d` for a digit `d`: the accumulator `ans = 10*ans + digit` adds
/// the digit at the units place, so it is lost to rounding once the
/// ulp exceeds twice the digit (no tie is possible then).
fn ld_add_digit(a: LD, d: u32) -> LD {
    if a.m == 0 {
        return LD::small(d as u64);
    }
    let bl = 128 - a.m.leading_zeros() as i32;
    if a.e + bl - 64 >= 5 {
        // half-ulp = 2^(e+bl-65) >= 16 > 9 >= d.
        return a;
    }
    // The sum is an integer of at most 69 bits; round it as the 80-bit
    // add would.
    debug_assert!(a.e >= 0 && a.e <= 4);
    ld_norm((a.m << a.e) + d as u128, 0)
}

/// The decimal case of `R_strtod5`: `text` is digits with at most one
/// `.`, then an optional `e`/`E` exponent (both already validated by the
/// scanner; no sign, no `0x` prefix — R's parser handles those).
fn decimal_to_f64(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut ans = LD::zero();
    let mut ndigits = 0usize;
    let mut expn: i32 = 0;
    let ten = LD::small(10);
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        ans = ld_add_digit(ld_mul(ans, ten), (bytes[i] - b'0') as u32);
        ndigits += 1;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            ans = ld_add_digit(ld_mul(ans, ten), (bytes[i] - b'0') as u32);
            ndigits += 1;
            expn -= 1;
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let sign = match bytes.get(i) {
            Some(b'-') => {
                i += 1;
                -1
            }
            Some(b'+') => {
                i += 1;
                1
            }
            _ => 1,
        };
        let mut v: i64 = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            v = if v < 9999 { v * 10 + (bytes[i] - b'0') as i64 } else { v };
            i += 1;
        }
        expn += sign * v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
    // "avoid unnecessary underflow for large negative exponents"
    if expn + (ndigits as i32) < -300 {
        for _ in 0..ndigits {
            ans = ld_div(ans, ten);
        }
        expn += ndigits as i32;
    }
    let mut p10 = ten;
    let mut fac = LD::small(1);
    if expn < -307 {
        // use underflow, not overflow
        let mut n = -expn;
        while n != 0 {
            if n & 1 == 1 {
                fac = ld_div(fac, p10);
            }
            n >>= 1;
            p10 = ld_mul(p10, p10);
        }
        ans = ld_mul(ans, fac);
    } else if expn < 0 {
        // positive powers are exact (in the 64-bit significand)
        let mut n = -expn;
        while n != 0 {
            if n & 1 == 1 {
                fac = ld_mul(fac, p10);
            }
            n >>= 1;
            p10 = ld_mul(p10, p10);
        }
        ans = ld_div(ans, fac);
    } else if ans.m != 0 {
        let mut n = expn;
        while n != 0 {
            if n & 1 == 1 {
                fac = ld_mul(fac, p10);
            }
            n >>= 1;
            p10 = ld_mul(p10, p10);
        }
        ans = ld_mul(ans, fac);
    }
    // (double) ans; the explicit `ans > DBL_MAX` check of the C code
    // gives the same infinity, values at the rounding boundary included.
    ld_to_f64(ans.m as u64, ans.e)
}
