//! Lexer and parser for R.
//!
//! The parser follows GNU R's `gram.y`: precedence, associativity, newline
//! handling and parse-time rewrites such as `->` and `|>` are reproduced so
//! that the resulting trees match what `parse()` returns in GNU R.

pub mod ast;
pub mod cfmt;
pub mod dump;
pub mod lexer;
pub mod parser;

pub use ast::{Arg, Const, Expr, Formal, ParseError, Span};
pub use parser::{parse_program, parse_program_with_warnings};

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
