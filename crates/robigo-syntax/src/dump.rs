//! Canonical S-expression dump of parsed code.
//!
//! `scripts/oracle-parse.R` produces the same format from GNU R's `parse()`,
//! which lets the test suite compare both parsers tree by tree. Dumps are
//! byte strings: strings keep their raw bytes (GNU R strings may hold
//! arbitrary bytes through `\xNN` escapes), and comparisons are byte-wise.

use crate::ast::*;
use crate::cfmt::fmt_g;
use crate::error_line_col;

pub fn dump(e: &Expr) -> Vec<u8> {
    let mut s = Vec::new();
    dump_into(e, &mut s);
    s
}

/// The canonical one-line text of a parse failure: `ERROR <line> <col>
/// <message>`, with GNU R's `(<input>:<line>:<col>)` suffix on messages
/// raised from parser actions.
pub fn dump_error(src: &str, e: &ParseError) -> Vec<u8> {
    let (line, col) = error_line_col(src, e);
    let mut s = format!("ERROR {} {} {}", line, col, e.message).into_bytes();
    if e.with_pos_suffix {
        s.extend_from_slice(format!(" (<input>:{}:{})", line, col).as_bytes());
    }
    s
}

/// Escape string bytes the way R's `encodeString(x, quote = '"')` does in
/// a UTF-8 locale: valid UTF-8 sequences pass through raw (except C1
/// controls, which print as `\u00xx`), and stray bytes print as `\xNN`.
pub fn encode_string(x: &[u8], quote: u8, out: &mut Vec<u8>) {
    out.push(quote);
    let mut i = 0;
    while i < x.len() {
        let b = x[i];
        match b {
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0b => out.extend_from_slice(b"\\v"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b if b == quote => out.extend_from_slice(&[b'\\', b]),
            b if b < 0x20 || b == 0x7f => {
                out.extend_from_slice(format!("\\{:03o}", b).as_bytes());
            }
            0x80..=0xff => {
                let len = b.len_utf8_seq();
                if let Some((c, used)) = decode_utf8(&x[i..], len) {
                    if (0x80..=0x9f).contains(&(c as u32)) {
                        out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
                    } else {
                        out.extend_from_slice(&x[i..i + used]);
                    }
                    i += used;
                    continue;
                }
                out.extend_from_slice(format!("\\x{:02x}", b).as_bytes());
            }
            b => out.push(b),
        }
        i += 1;
    }
    out.push(quote);
}

/// The length a UTF-8 sequence starting with `b` would have (0: invalid
/// start byte).
trait Utf8Seq {
    fn len_utf8_seq(self) -> usize;
}

impl Utf8Seq for u8 {
    fn len_utf8_seq(self) -> usize {
        match self {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        }
    }
}

/// Decode one strictly valid UTF-8 sequence of `len` bytes from the front
/// of `x`.
fn decode_utf8(x: &[u8], len: usize) -> Option<(char, usize)> {
    let seq = x.get(..len)?;
    let s = std::str::from_utf8(seq).ok()?;
    let c = s.chars().next()?;
    (c.len_utf8() == len).then_some((c, len))
}

pub fn dump_double(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x == f64::INFINITY {
        "Inf".into()
    } else if x == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        fmt_g(x, 17, false, false)
    }
}

fn dump_const(c: &Const, s: &mut Vec<u8>) {
    match c {
        Const::Null => s.extend_from_slice(b"NULL"),
        Const::Lgl(None) => s.extend_from_slice(b"lgl:NA"),
        Const::Lgl(Some(b)) => s.extend_from_slice(if *b { b"lgl:TRUE" } else { b"lgl:FALSE" }),
        Const::Int(None) => s.extend_from_slice(b"int:NA"),
        Const::Int(Some(i)) => s.extend_from_slice(format!("int:{}", i).as_bytes()),
        Const::DblNa => s.extend_from_slice(b"dbl:NA"),
        Const::Dbl(x) => {
            s.extend_from_slice(b"dbl:");
            s.extend_from_slice(dump_double(*x).as_bytes());
        }
        Const::CplxNa => s.extend_from_slice(b"cplx:NA"),
        Const::Cplx(re, im) => {
            s.extend_from_slice(b"cplx:");
            s.extend_from_slice(dump_double(*re).as_bytes());
            s.push(b',');
            s.extend_from_slice(dump_double(*im).as_bytes());
        }
        Const::Str(None) => s.extend_from_slice(b"str:NA"),
        Const::Str(Some(x)) => {
            s.extend_from_slice(b"str:");
            encode_string(x, b'"', s);
        }
    }
}

fn dump_into(e: &Expr, s: &mut Vec<u8>) {
    match e {
        Expr::Const(c, _) => dump_const(c, s),
        Expr::Sym(name, _) => {
            s.push(b'`');
            s.extend_from_slice(name.as_bytes());
            s.push(b'`');
        }
        // GNU R's tree holds the string constant `"_"` here.
        Expr::Placeholder(_) => {
            s.extend_from_slice(b"str:\"_\"");
        }
        Expr::Call { fun, args, .. } => {
            s.push(b'(');
            dump_into(fun, s);
            for a in args {
                s.push(b' ');
                dump_arg(a.name.as_deref(), a.value.as_ref(), s);
            }
            s.push(b')');
        }
        Expr::Function { formals, body, .. } => {
            s.extend_from_slice(b"(`function` (pairlist");
            for f in formals {
                s.push(b' ');
                dump_arg(Some(&f.name), f.default.as_ref(), s);
            }
            s.extend_from_slice(b") ");
            dump_into(body, s);
            s.extend_from_slice(b" NULL)");
        }
    }
}

fn dump_arg(name: Option<&str>, value: Option<&Expr>, s: &mut Vec<u8>) {
    if let Some(n) = name {
        s.extend_from_slice(n.as_bytes());
        s.push(b'=');
    }
    match value {
        Some(v) => dump_into(v, s),
        None => s.extend_from_slice(b"<missing>"),
    }
}
