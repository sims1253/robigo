//! Lexer and parser for R.
//!
//! The parser follows GNU R's `gram.y`: precedence, associativity, newline
//! handling and parse-time rewrites such as `->` and `|>` are reproduced so
//! that the resulting trees match what `parse()` returns in GNU R.

pub mod arena;
pub mod ast;
pub mod cfmt;
pub mod dump;
pub mod lexer;
pub mod parser;

pub use arena::Arena;
pub use ast::{Arg, Const, ErrorExpr, ErrorStyle, Expr, Formal, ParseError, Span};
pub use parser::{ConsoleStatus, parse_console, parse_program, parse_program_with_warnings};

/// Convert a byte offset into a 1-based (line, column) pair, counting the
/// way GNU R's lexer does (gram.c:xxgetc): columns count characters, not
/// bytes, and a tab advances to the next multiple of 8. The result is the
/// column of the character at `offset` (so it is one past the last counted
/// character).
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
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
/// `xxcolno` after that character was read), or, at end of input, the
/// position after the virtual trailing newline that the lexer appends.
pub fn error_line_col(src: &str, e: &ParseError) -> (usize, usize) {
    if let Some(fixed) = e.fixed_line_col {
        return fixed;
    }
    if e.at_eof {
        let mut line = src.matches('\n').count() + 1;
        if !src.is_empty() && !src.ends_with('\n') {
            line += 1;
        }
        return (line, 0);
    }
    line_col(src, e.offset)
}

/// `PARSE_CONTEXT_SIZE` (R: gram.c): the parse error context keeps the
/// last this many bytes read.
pub const PARSE_CONTEXT_SIZE: usize = 256;

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
/// ring buffer works in bytes, so a slice can split a multi-byte
/// character; the pieces are shown lossily instead of panicking.
pub fn parse_context(src: &str, end: usize) -> Vec<String> {
    let end = end.min(src.len());
    let start = floor_char_boundary(src, end.saturating_sub(PARSE_CONTEXT_SIZE));
    let raw = String::from_utf8_lossy(&src.as_bytes()[start..end]);
    let mut lines: Vec<String> = raw
        .split('\n')
        .map(|line| {
            let mut expanded = String::new();
            for c in line.chars() {
                if expanded.len() >= TAB_EXPAND_MAX {
                    break;
                }
                if c == '\t' {
                    loop {
                        expanded.push(' ');
                        if expanded.len() % 8 == 0 {
                            break;
                        }
                    }
                } else {
                    expanded.push(c);
                }
            }
            expanded
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
pub fn condition_message(src: &str, srcname: &str, e: &ParseError) -> String {
    let (line, col) = error_line_col(src, e);
    format!("{} ({}:{}:{})", e.message, srcname, line, col)
}

/// `parseError(call, linenum)` with a non-zero line number (R:
/// main/source.c): `parse()` and `str2lang()` report a grammar error from
/// inside the builtin, before `R_ParseError` is reset, so the message
/// starts with `<file>:<line>:<col>:` and is followed by the context lines
/// and a caret.
pub fn format_parse_error(src: &str, srcname: &str, e: &ParseError) -> String {
    let (line, col) = error_line_col(src, e);
    let prefix = if srcname.is_empty() { String::new() } else { format!("{srcname}:") };
    let mut msg = format!("{}{}:{}: {}", prefix, line, col, e.message);
    let context = parse_context(src, e.end);
    let ctx_line = parse_context_line(src, e.end);
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
