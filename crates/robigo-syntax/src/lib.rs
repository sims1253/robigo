//! Lexer and parser for R.
//!
//! The parser follows GNU R's `gram.y`: precedence, associativity, newline
//! handling and parse-time rewrites such as `->` and `|>` are reproduced so
//! that the resulting trees match what `parse()` returns in GNU R.

#![forbid(unsafe_code)]

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

pub mod arena;
pub mod ast;
pub mod cfmt;
pub mod dump;
pub mod lexer;
pub mod parse_data;
pub mod parser;

pub use lexer::is_valid_name;

pub use arena::Arena;
pub use ast::{Arg, ConditionValue, Const, ErrorExpr, ErrorStyle, Expr, Formal, ParseError, Span};
pub use parser::{
    ConnStatus, ConsoleStatus, parse_conn_statement, parse_console, parse_program, parse_program_with_warnings,
};

/// The byte that stands in, in [`Source::text`], for each byte of the raw
/// source that is not part of valid UTF-8 (ASCII SUB, "substitute").
pub const INVALID_BYTE: u8 = 0x1a;

/// Source text as GNU R's lexer reads it: bytes, which need not be valid
/// UTF-8 (a script with a stray latin1 byte, `parse(text = "\xff")`).
///
/// The lexer and everything downstream of it slice the source as `str`,
/// so `text` is the raw bytes with every byte of an invalid sequence
/// replaced by [`INVALID_BYTE`] ([`lexable`]): same length, same offsets.
/// The lexer looks at `raw` only where it meets a replaced byte, to apply
/// gram.y's `mbcs_get_next` (the "invalid multibyte character in parser"
/// error, or end of input when the character's bytes run out).
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub text: &'a str,
    raw: &'a [u8],
    known_to_be_utf8: bool,
}

impl<'a> Source<'a> {
    /// `text` must be [`lexable`]`(raw)`.
    pub fn new(text: &'a str, raw: &'a [u8]) -> Self {
        assert_eq!(text.len(), raw.len(), "Source::new: text is not lexable(raw)");
        debug_assert_eq!(&*lexable(raw), text, "Source::new: text is not lexable(raw)");
        Source { text, raw, known_to_be_utf8: true }
    }

    /// GNU R's `known_to_be_utf8` for this parse, true by default (a UTF-8
    /// session). `parse()`, `str2lang()` and `str2expression()` clear it
    /// for text with a non-ASCII string of unknown encoding, and for a
    /// file read without `encoding = "UTF-8"` (R: source.c:do_parse,
    /// coerce.c:do_str2lang). Error positions then count UTF-8
    /// continuation bytes as columns too.
    pub fn known_to_be_utf8(self, known: bool) -> Self {
        Source { known_to_be_utf8: known, ..self }
    }

    /// The source byte at `i`, when `text` has [`INVALID_BYTE`] there in
    /// its place (a genuine SUB byte is itself).
    #[inline]
    pub(crate) fn invalid_byte(&self, i: usize) -> Option<u8> {
        let b = *self.raw.get(i)?;
        (b != self.text.as_bytes()[i]).then_some(b)
    }

    /// The source bytes.
    pub fn raw(&self) -> &'a [u8] {
        self.raw
    }

    /// Whether any byte was replaced (the lexer's slow paths are off
    /// otherwise).
    pub(crate) fn has_invalid(&self) -> bool {
        !std::ptr::eq(self.text.as_bytes(), self.raw) && self.text.as_bytes() != self.raw
    }
}

impl<'a> From<&'a str> for Source<'a> {
    fn from(text: &'a str) -> Self {
        Source { text, raw: text.as_bytes(), known_to_be_utf8: true }
    }
}

impl<'a> From<&'a String> for Source<'a> {
    fn from(text: &'a String) -> Self {
        Source::from(text.as_str())
    }
}

/// The text the lexer reads for `raw` (see [`Source`]): `raw` itself when
/// it is valid UTF-8, otherwise a copy with each byte of every invalid
/// sequence replaced by [`INVALID_BYTE`].
pub fn lexable(raw: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(raw) {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) => {
            let mut out = Vec::with_capacity(raw.len());
            for chunk in raw.utf8_chunks() {
                out.extend_from_slice(chunk.valid().as_bytes());
                out.extend(std::iter::repeat_n(INVALID_BYTE, chunk.invalid().len()));
            }
            std::borrow::Cow::Owned(String::from_utf8(out).expect("lexable: replaced bytes are ASCII"))
        }
    }
}

impl Source<'_> {
    /// gram.y's `(xxlineno, xxcolno)` after `xxgetc` has read
    /// `raw[..end]` from the start of the parse: a newline starts a new
    /// line at column 0, a tab advances to the next multiple of 8, and
    /// UTF-8 continuation bytes do not advance the column when the input
    /// is known to be UTF-8 (R: gram.y:xxgetc).
    pub(crate) fn r_line_col(&self, end: usize) -> (usize, usize) {
        let (mut line, mut col) = (1usize, 0usize);
        for &b in &self.raw[..end.min(self.raw.len())] {
            if b == b'\n' {
                line += 1;
                col = 0;
                continue;
            }
            if !self.known_to_be_utf8 || !(0x80..0xc0).contains(&b) {
                col += 1;
            }
            if b == b'\t' {
                col = (col + 7) & !7;
            }
        }
        (line, col)
    }
}

/// `utf8clen` (R: util.c): the length of the UTF-8 sequence a byte leads,
/// 1 for ASCII and for a stray continuation byte, up to 6 for the old
/// 5- and 6-byte forms.
pub(crate) fn utf8clen(b: u8) -> usize {
    match b {
        0x00..=0xbf => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        0xf8..=0xfb => 5,
        _ => 6,
    }
}

/// `finish_mbcs_in_parse_context` (R: gram.y), which `yyerror` runs
/// unless the lexer has hit the end of input: when the parse context, the
/// `end` bytes read so far, stops inside a character, the character's
/// remaining bytes are added to it from the input. Returns the context's
/// new end, or `None` when the input runs out first, which raises "invalid
/// multibyte character in parser" (`add_mbcs_byte_to_parse_context`).
/// Only a context still entirely in R's ring buffer is completed.
pub(crate) fn finish_mbcs(raw: &[u8], end: usize) -> Option<usize> {
    if end >= PARSE_CONTEXT_SIZE {
        return Some(end);
    }
    let mut i = 0;
    while i < end {
        let c = raw[i];
        if c >= 0x80 {
            i += utf8clen(c) - 1;
            if i >= end {
                return (i < raw.len()).then_some(i + 1);
            }
        }
        i += 1;
    }
    Some(end)
}

/// Convert a byte offset into a 1-based (line, column) pair, counting the
/// way GNU R's lexer does (gram.c:xxgetc): columns count characters, not
/// bytes, and a tab advances to the next multiple of 8. The result is the
/// column of the character at `offset` (so it is one past the last counted
/// character).
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    // An error can point at the byte after a lead byte, as gram.c counts
    // bytes: the character it falls in is the one being reported.
    let before = &src[..floor_char_boundary(src, offset)];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut col = 0usize;
    for c in before[line_start..].chars() {
        col = if c == '\t' { (col + 1 + 7) & !7 } else { col + 1 };
    }
    (line, col + 1)
}

/// The position of a parse error as GNU R reports it: the column of the
/// offending token's first character (`lloc.first_column`, which is
/// `xxcolno` after that character was read, so a column per byte unless
/// the input is known to be UTF-8), or, at end of input, the position
/// after the virtual trailing newline that the lexer appends.
pub fn error_line_col<'a>(src: impl Into<Source<'a>>, e: &ParseError) -> (usize, usize) {
    let src = src.into();
    if let Some(fixed) = e.fixed_line_col {
        return fixed;
    }
    if e.at_eof {
        let text = src.text;
        let mut line = text.matches('\n').count() + 1;
        if !text.is_empty() && !text.ends_with('\n') {
            line += 1;
        }
        return (line, 0);
    }
    if src.known_to_be_utf8 {
        line_col(src.text, e.offset)
    } else {
        let (line, col) = src.r_line_col(e.offset);
        (line, col + 1)
    }
}

/// `PARSE_CONTEXT_SIZE` (R: gram.c): the parse error context keeps the
/// last this many bytes read.
pub const PARSE_CONTEXT_SIZE: usize = 256;

/// The source name a parse condition reports when there is no file (R:
/// gram.y's `R_ParseErrorSrcfile` default, `"<input>"`).
pub const CONSOLE_SRCNAME: &str = "<input>";

/// `tabExpand`'s per-line cap (R: main/source.c).
const TAB_EXPAND_MAX: usize = 192;

fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The last [`PARSE_CONTEXT_SIZE`] bytes before `end`, tab-expanded like
/// `tabExpand` and split into lines like `getParseContext` (R:
/// main/source.c), which also drops an empty line after a newline. R's
/// ring buffer works in bytes and prints them as they are; a byte that is
/// not valid UTF-8 shows as U+FFFD here.
pub fn parse_context(src: &[u8], end: usize) -> Vec<String> {
    let end = end.min(src.len());
    let mut start = end.saturating_sub(PARSE_CONTEXT_SIZE);
    // Start at a character, as a `str` slice would.
    while start > 0 && (0x80..0xc0).contains(&src[start]) {
        start -= 1;
    }
    // R: source.c:getParseContext / tabExpand. Expand each line in its
    // own 200-byte buffer, stopping input once 192 bytes are written.
    let mut lines: Vec<String> = src[start..end]
        .split(|b| *b == b'\n')
        .map(|line| {
            let mut expanded = Vec::new();
            for &b in line {
                if expanded.len() >= TAB_EXPAND_MAX {
                    break;
                }
                if b == b'\t' {
                    loop {
                        expanded.push(b' ');
                        if expanded.len().is_multiple_of(8) {
                            break;
                        }
                    }
                } else {
                    expanded.push(b);
                }
            }
            String::from_utf8_lossy(&expanded).into_owned()
        })
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// `R_ParseContextLine` (R: gram.y:xxgetc, source.c:getParseContext): the
/// line of the last character read. A newline belongs to the line it ends,
/// which is exactly the decrement `getParseContext` applies when it drops
/// the empty line after a trailing newline.
pub fn parse_context_line(src: &str, end: usize) -> usize {
    if end == 0 {
        return 0;
    }
    line_col(src, floor_char_boundary(src, end.min(src.len()) - 1)).0
}

/// `raiseParseError`'s message (R: gram.y): the message text with the
/// `(<file>:<line>:<col>)` suffix that the format strings append.
pub fn condition_message<'a>(src: impl Into<Source<'a>>, srcname: &str, e: &ParseError) -> String {
    let (line, col) = error_line_col(src, e);
    format!("{} ({}:{}:{})", e.message, srcname, line, col)
}

/// `parseError(call, linenum)` with a non-zero line number (R:
/// main/source.c): `parse()` and `str2lang()` report a grammar error from
/// inside the builtin, before `R_ParseError` is reset, so the message
/// starts with `<file>:<line>:<col>:` and is followed by the context lines
/// and a caret.
pub fn format_parse_error<'a>(src: impl Into<Source<'a>>, srcname: &str, e: &ParseError) -> String {
    let src = src.into();
    let (line, col) = error_line_col(src, e);
    let prefix = if srcname.is_empty() { String::new() } else { format!("{srcname}:") };
    let mut msg = format!("{}{}:{}: {}", prefix, line, col, e.message);
    let context = parse_context(src.raw, e.end);
    let ctx_line = parse_context_line(src.text, e.end);
    match context.len() {
        0 => {}
        1 => {
            // `width = snprintf(buffer, 10, "%d: ", R_ParseContextLine)`
            let width = ctx_line.to_string().len() + 2;
            msg.push_str(&format!("\n{}: {}\n{}^", ctx_line, context[0], " ".repeat(width + col)));
        }
        len => {
            // `width = snprintf(buffer, 10, "%d:", R_ParseContextLine)`
            let width = ctx_line.to_string().len() + 1;
            msg.push_str(&format!(
                "\n{}: {}\n{}: {}\n{}^",
                ctx_line - 1,
                context[len - 2],
                ctx_line,
                context[len - 1],
                " ".repeat(width + col)
            ));
        }
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(raw: &[u8], f: impl FnOnce(Source<'_>)) {
        let text = lexable(raw);
        f(Source::new(&text, raw));
    }

    #[test]
    fn lexable_replaces_each_invalid_byte() {
        assert!(matches!(lexable("x <- é".as_bytes()), std::borrow::Cow::Borrowed(_)));
        assert_eq!(lexable(b"a\xffb\xe2\x82"), "a\x1ab\x1a\x1a");
        source(b"a\xff\x1a", |s| {
            assert_eq!(s.invalid_byte(1), Some(0xff));
            // A genuine SUB in the source is not an invalid byte.
            assert_eq!(s.invalid_byte(2), None);
            assert!(s.has_invalid());
        });
        source(b"a\x1a", |s| assert!(!s.has_invalid()));
    }

    /// gram.y:xxgetc: continuation bytes are columns only when the input
    /// is not known to be UTF-8; tabs go to the next multiple of 8.
    #[test]
    fn r_line_col_counts_like_xxgetc() {
        let raw = "é\tx\nab".as_bytes();
        source(raw, |s| {
            assert_eq!(s.r_line_col(2), (1, 1));
            assert_eq!(s.r_line_col(3), (1, 8));
            assert_eq!(s.r_line_col(5), (2, 0));
            assert_eq!(s.r_line_col(7), (2, 2));
            let s = s.known_to_be_utf8(false);
            assert_eq!(s.r_line_col(2), (1, 2));
            assert_eq!(s.r_line_col(3), (1, 8));
        });
    }

    #[test]
    fn finish_mbcs_completes_the_last_character() {
        // Ends after a whole character, or in ASCII: unchanged.
        assert_eq!(finish_mbcs("x€".as_bytes(), 4), Some(4));
        assert_eq!(finish_mbcs(b"x+", 1), Some(1));
        // Ends after the lead byte of `€`: its two other bytes join.
        assert_eq!(finish_mbcs("x€ +".as_bytes(), 2), Some(4));
        // The input runs out inside the character: invalidMBCS.
        assert_eq!(finish_mbcs(b"1e\xff", 3), None);
        assert_eq!(finish_mbcs(b"1e\xff\n", 3), None);
        assert_eq!(finish_mbcs(b"1e\xc3\n", 3), Some(4));
        // Past R's ring buffer: left alone.
        let long = [b"x".repeat(PARSE_CONTEXT_SIZE), b"\xe2".to_vec()].concat();
        assert_eq!(finish_mbcs(&long, long.len()), Some(long.len()));
    }

    /// Positions and messages checked against GNU R 4.5.0's
    /// `parse(text = s, encoding = "UTF-8")` (each element ends in "\n").
    #[test]
    fn invalid_bytes_parse_like_gram_y() {
        let parse = |raw: &[u8]| {
            let text = lexable(raw);
            parse_program(&Arena::new(), Source::new(&text, raw)).map(|e| e.len())
        };
        let mbcs = |raw: &[u8], at: (usize, usize)| {
            let e = parse(raw).unwrap_err();
            assert_eq!((e.subclass, e.message.as_str()), ("invalidMBCS", "invalid multibyte character in parser"));
            assert_eq!(e.fixed_line_col, Some(at), "{raw:?}");
        };
        mbcs(b"x <- 1\ny\xff + 1\n", (3, 0));
        mbcs(b"1e\xff\n", (1, 3));
        mbcs(b"x + \xe2\x82\n", (2, 0));
        // The character's bytes run out: end of input, after an
        // expression an error, at the start a clean end.
        assert_eq!(parse(b"zz \xff\n").unwrap_err().message, "unexpected end of input");
        assert_eq!(parse(b"\xc3\xa9\xff\n").unwrap_err().message, "unexpected end of input");
        assert_eq!(parse(b"\t\xff\n").ok(), Some(0));
    }
}
