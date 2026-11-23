//! Source references: gram.y's srcrefs, made when `keep.source` is TRUE.
//!
//! GNU R's lexer keeps a position state (`ParseState`'s `xxlineno`,
//! `xxcolno`, `xxbyteno` and `xxparseno`) and updates it in `xxgetc`, one
//! character at a time. A token's location is that state after its first
//! character was read (its first fields, gram.y's `token()`) and after its
//! last (its last fields, `setlastloc`). An expression's location is
//! bison's default: the first fields of its first token and the last
//! fields of its last. robigo-syntax keeps a byte span per node, so the
//! state after each byte of the source ([`Positions`]) gives every field
//! exactly: `xxgetc`'s rules are ported here, and a node's srcref is the
//! state after its first byte and after its last.
//!
//! None of this runs when `keep.source` is FALSE.

#![allow(unsafe_code)]

use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use robigo_rtcore::Cx;

/// gram.y's `ParseState` position after a character is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pos {
    /// `xxlineno`: the line, as `#line` directives number it.
    pub line: i32,
    /// `xxbyteno`: bytes read on the line.
    pub byte: i32,
    /// `xxcolno`: the display column (tabs to multiples of 8).
    pub col: i32,
    /// `xxparseno`: lines read ("it counts parsed lines, not official
    /// lines"): `#line` does not change it.
    pub parsed: i32,
}

/// A `#line N "file"` directive's filename, in effect from byte `from` on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Alias {
    pub from: usize,
    pub filename: Vec<u8>,
}

/// The position state after each byte of a source text.
#[derive(Debug)]
pub(crate) struct Positions {
    /// `after[i]`: the state after byte `i` was read.
    after: Vec<Pos>,
    /// The state after the whole text (or the start state, for an empty one).
    end: Pos,
    /// The `#line` directives' filenames, in order.
    pub aliases: Vec<Alias>,
}

/// `R_InitSrcRefState`'s start: line 1, column 0, byte 0, parsed line 1.
const START: Pos = Pos { line: 1, byte: 0, col: 0, parsed: 1 };

impl Positions {
    /// The state after each byte of `src` (as the lexer reads it; `in_string`
    /// says whether a byte lies inside a string constant, where a `#` is no
    /// comment and so no `#line` directive). `known_utf8`: gram.y's
    /// `known_to_be_utf8`.
    // R: gram.y:xxgetc, SkipComment, processLineDirective
    pub(crate) fn new(src: &[u8], known_utf8: bool, in_string: impl Fn(usize) -> bool) -> Positions {
        let mut after = Vec::with_capacity(src.len());
        let mut aliases = Vec::new();
        let mut s = START;
        // a `#line` directive read: its line number, applied once its
        // newline is read
        let mut pending: Option<i32> = None;
        let mut i = 0;
        while i < src.len() {
            let c = src[i];
            if c == b'\n' {
                s.line += 1;
                s.col = 0;
                s.byte = 0;
                s.parsed += 1;
                if let Some(n) = pending.take() {
                    // processLineDirective: "we don't change xxparseno here:
                    // it counts parsed lines, not official lines"
                    s.line = n;
                }
            } else {
                // "We only advance the column for the 1st byte in UTF-8, so
                // handle later bytes specially"
                if !known_utf8 || c < 0x80 || 0xC0 <= c {
                    s.col += 1;
                }
                s.byte += 1;
            }
            if c == b'\t' {
                s.col = (s.col + 7) & !7;
            }
            after.push(s);
            // SkipComment: `maybeLine = (ParseState.xxcolno == 1)` once the
            // `#` is read; then "line", a number and an optional "file"
            if c == b'#' && s.col == 1 && !in_string(i) {
                if let Some((n, file, _)) = line_directive(&src[i + 1..]) {
                    pending = Some(n);
                    if let Some(f) = file {
                        let nl = src[i..].iter().position(|&b| b == b'\n').map_or(src.len(), |k| i + k + 1);
                        aliases.push(Alias { from: nl, filename: f.to_vec() });
                    }
                }
            }
            i += 1;
        }
        let end = after.last().copied().unwrap_or(START);
        Positions { after, end, aliases }
    }

    /// The state after byte `i` (`i` within the text).
    pub(crate) fn after(&self, i: usize) -> Pos {
        self.after[i]
    }

    /// The state after the whole text.
    pub(crate) fn end(&self) -> Pos {
        self.end
    }

    /// makeSrcref's eight fields for the bytes `first..=last`.
    // R: gram.y:makeSrcref
    pub(crate) fn fields(&self, first: usize, last: usize) -> [i32; 8] {
        let (f, l) = (self.after(first), self.after(last));
        [f.line, f.byte, l.line, l.byte, f.col, l.col, f.parsed, l.parsed]
    }

    /// attachSrcrefs' `wholeSrcref`: from the start (line 1, byte 0,
    /// column 0, parsed line 1) to `last`.
    // R: gram.y:attachSrcrefs
    pub(crate) fn whole(last: Pos) -> [i32; 8] {
        [1, 0, last.line, last.byte, 0, last.col, 1, last.parsed]
    }
}

/// After a `#` that starts a line: `line N`, optionally `"file"`, as
/// processLineDirective reads it: (N, the file, bytes taken), or None
/// when it is no directive ("#line" not followed by a number).
// R: gram.y:SkipComment, processLineDirective
fn line_directive(rest: &[u8]) -> Option<(i32, Option<&[u8]>, usize)> {
    let r = rest.strip_prefix(b"line")?;
    let mut j = 0;
    let space = |b: u8| b == b' ' || b == b'\t' || b == b'\x0c';
    while j < r.len() && space(r[j]) {
        j += 1;
    }
    if j >= r.len() || !r[j].is_ascii_digit() {
        return None;
    }
    // NumericValue, then atoi(yytext): the leading digits
    let start = j;
    while j < r.len() && r[j].is_ascii_digit() {
        j += 1;
    }
    let n: i32 = std::str::from_utf8(&r[start..j]).ok()?.parse().ok()?;
    while j < r.len() && !matches!(r[j], b'\n' | b'"') && !r[j].is_ascii_alphanumeric() {
        j += 1;
    }
    let file = if j < r.len() && r[j] == b'"' {
        let s = j + 1;
        let e = r[s..].iter().position(|&b| b == b'"' || b == b'\n').map_or(r.len(), |k| s + k);
        Some(&r[s..e])
    } else {
        None
    };
    Some((n, file, j))
}

/// makeSrcref: the INTSXP of `v`, its "srcfile" attribute `srcfile` and
/// class "srcref". Under `&Cx`: allocates, never collects (a refusal is
/// retried by the caller).
///
/// # Safety
/// `srcfile` is live; `class` is the STRSXP "srcref".
// R: gram.y:makeSrcref
pub(crate) unsafe fn make(cx: &Cx, v: [i32; 8], srcfile: Sexp, class: Sexp) -> Result<Sexp, NeedGc> {
    // SAFETY: the caller's contract; fresh objects of this heap.
    unsafe {
        let x = cx.alloc_vector_raw(INTSXP, 8)?;
        let p = x.std_data() as *mut i32;
        for (k, &f) in v.iter().enumerate() {
            *p.add(k) = f;
        }
        let nil = cx.nil().sexp();
        let srcfile_sym = cx.install(b"srcfile")?.as_ref().sexp();
        let class_sym = cx.install(b"class")?.as_ref().sexp();
        let attrs = cx.alloc_node_raw(LISTSXP, class, nil, class_sym)?;
        let attrs = cx.alloc_node_raw(LISTSXP, srcfile, attrs, srcfile_sym)?;
        x.set_attrib(attrs);
        x.set_object(true);
        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(src: &str) -> Positions {
        Positions::new(src.as_bytes(), true, |_| false)
    }

    /// GNU R 4.6.2's srcrefs for `f <- function(x) {\n  y <- x + 1\n  y * 2\n}`
    /// (recorded: parse(text = , keep.source = TRUE)).
    #[test]
    fn the_fields_are_gnu_rs_for_a_function_and_its_braces() {
        let src = "f <- function(x) {\n  y <- x + 1\n  y * 2\n}\ng <- 3\n";
        let p = pos(src);
        // the first statement, `f` to `}`
        let close = src.find('}').unwrap();
        assert_eq!(p.fields(0, close), [1, 1, 4, 1, 1, 1, 1, 4]);
        // the `{` token, and `y <- x + 1`
        let open = src.find('{').unwrap();
        assert_eq!(p.fields(open, open), [1, 18, 1, 18, 18, 18, 1, 1]);
        let y = src.find("y <-").unwrap();
        assert_eq!(p.fields(y, y + "y <- x + 1".len() - 1), [2, 3, 2, 12, 3, 12, 2, 2]);
        // the `{`'s wholeSrcref ends at its `}`, the text's after its last newline
        assert_eq!(Positions::whole(p.after(close)), [1, 0, 4, 1, 0, 1, 1, 4]);
        assert_eq!(Positions::whole(p.end()), [1, 0, 6, 0, 0, 0, 1, 6]);
    }

    /// A tab rounds the column to a multiple of 8 (recorded: `\tx <- 1`
    /// starts at byte 2, column 9).
    #[test]
    fn a_tab_rounds_the_column() {
        let src = "if (TRUE) {\n\tx <- 1\n}\n";
        let p = pos(src);
        let x = src.find('x').unwrap();
        assert_eq!(p.fields(x, x + "x <- 1".len() - 1), [2, 2, 2, 7, 9, 14, 2, 2]);
    }

    /// `#line 40 "other.R"`: the next line is 40, the parsed line goes on
    /// (recorded: 40 1 40 6 1 6 3 3), and the file is an alias from there.
    #[test]
    fn a_line_directive_renumbers_the_lines_not_the_parsed_ones() {
        let src = "a <- 1\n#line 40 \"other.R\"\nb <- 2\nc <- 3\n";
        let p = pos(src);
        let b = src.find("b <-").unwrap();
        assert_eq!(p.fields(b, b + 5), [40, 1, 40, 6, 1, 6, 3, 3]);
        let c = src.find("c <-").unwrap();
        assert_eq!(p.fields(c, c + 5), [41, 1, 41, 6, 1, 6, 4, 4]);
        assert_eq!(Positions::whole(p.end()), [1, 0, 42, 0, 0, 0, 1, 5]);
        assert_eq!(p.aliases, vec![Alias { from: b, filename: b"other.R".to_vec() }]);
    }

    /// A `#` inside a string, or not first on its line, is no directive.
    #[test]
    fn only_a_comment_first_on_its_line_is_a_directive() {
        let src = "x <- \"\n#line 9\"\ny <- 1 #line 9\n";
        let open = src.find('"').unwrap();
        let close = src.rfind('"').unwrap();
        let p = Positions::new(src.as_bytes(), true, |i| i > open && i < close);
        assert!(p.aliases.is_empty());
        let y = src.find("y <-").unwrap();
        assert_eq!(p.after(y).line, 3);
    }
}
