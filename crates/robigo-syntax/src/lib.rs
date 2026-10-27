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

/// Convert a byte offset into a 1-based (line, column) pair.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map(|i| offset - i).unwrap_or(offset + 1);
    (line, col)
}
