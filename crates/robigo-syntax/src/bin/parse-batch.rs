//! Parse every source in a manifest and dump the parse trees in the
//! oracle's canonical format, one physical line per source:
//! `id \t dump \x01 dump ...` or `id \t ERROR <line> <col> <message>`.
//!
//! Manifest lines are `id \t path \t enc`, with `enc` one of
//! `u` (UTF-8/ASCII, parsed as is) and `l` (Latin-1, transcoded to UTF-8
//! first). Output goes to the file named by the second argument, or stdout
//! for `-`. This is the tool behind `scripts/parse-corpus.py` and the
//! parse-throughput measurements.
//!
//! `--parse-only` skips the dump and folds a checksum over the trees, for
//! throughput runs: same parsing work, no formatting in the measurement.

use std::io::{BufWriter, Write};
use std::path::Path;

use robigo_syntax::dump::{dump, dump_error};
use robigo_syntax::parse_program;

fn transcode_latin1(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(b as char); // Latin-1 maps bytes to U+0000..U+00FF
    }
    s
}

/// R's oracle parses files through a text connection, whose newline
/// translation turns CR and CRLF into LF before the parser sees a single
/// byte; do the same here so both sides parse one stream.
fn strip_cr(raw: &[u8]) -> Vec<u8> {
    if !raw.contains(&b'\r') {
        return raw.to_vec();
    }
    let mut t = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' {
            t.push(b'\n');
            if i + 1 < raw.len() && raw[i + 1] == b'\n' {
                i += 1;
            }
        } else {
            t.push(raw[i]);
        }
        i += 1;
    }
    t
}

/// A cheap checksum over the trees keeps the optimizer from eliding the
/// work in throughput runs: O(nodes) of arithmetic, no formatting.
fn tree_checksum(exprs: &[robigo_syntax::Expr], seed: u64) -> u64 {
    let mut h = seed;
    let mut stack: Vec<&robigo_syntax::Expr> = exprs.iter().collect();
    while let Some(e) = stack.pop() {
        h = h.wrapping_mul(0x100000001b3)
            .wrapping_add(((e.span().start as u64) << 32) ^ e.span().end as u64);
        match e {
            robigo_syntax::Expr::Call { fun, args, .. } => {
                stack.push(fun);
                for a in args {
                    if let Some(v) = &a.value {
                        stack.push(v);
                    }
                }
            }
            robigo_syntax::Expr::Function { body, .. } => stack.push(body),
            _ => {}
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let parse_only = args.iter().any(|a| a == "--parse-only");
    let rest: Vec<&str> = args[1..].iter().filter(|a| **a != "--parse-only").map(|a| a.as_str()).collect();
    if rest.len() < 2 {
        eprintln!("usage: parse-batch [--parse-only] MANIFEST OUTFILE|-");
        std::process::exit(2);
    }
    let manifest = std::fs::read_to_string(rest[0]).expect("read manifest");
    let stdout = std::io::stdout();
    let file = std::fs::File::create(rest[1]).ok();
    let mut out: Box<dyn Write> = if rest[1] == "-" {
        Box::new(BufWriter::new(stdout.lock()))
    } else {
        match file {
            Some(f) => Box::new(BufWriter::new(f)),
            None => Box::new(BufWriter::new(stdout.lock())),
        }
    };
    let mut checksum: u64 = 0;
    let mut n = 0u64;
    let tokenize_only = std::env::var("ROBIGO_PARSE_TOKENIZE_ONLY").is_ok();
    let mut bytes_out: Vec<u8> = Vec::new();
    for line in manifest.lines() {
        let Some((id, rest)) = line.split_once('\t') else { continue };
        // Columns: path, enc, sha256-prefix. The enc is exactly the
        // second field; a trailing hash must not bleed into it.
        let (path, enc) = match rest.split_once('\t') {
            Some((p, rest2)) => (p, rest2.split('\t').next().unwrap_or("u")),
            None => (rest, "u"),
        };
        let Ok(raw) = std::fs::read(Path::new(path)) else {
            continue;
        };
        let raw = strip_cr(&raw);
        let src = if enc == "l" { transcode_latin1(&raw) } else { String::from_utf8_lossy(&raw).into_owned() };
        n += 1;
        if tokenize_only {
            use robigo_syntax::lexer::Lexer;
            match Lexer::new(&src).tokenize() {
                Ok((toks, _)) => checksum = checksum.wrapping_mul(31).wrapping_add(toks.len() as u64),
                Err(e) => checksum = checksum.wrapping_add(e.offset as u64),
            }
            continue;
        }
        match parse_program(&src) {
            Ok(exprs) => {
                if parse_only {
                    checksum = tree_checksum(&exprs, checksum);
                    continue;
                }
                bytes_out.clear();
                bytes_out.extend_from_slice(id.as_bytes());
                bytes_out.push(b'\t');
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        bytes_out.push(1);
                    }
                    bytes_out.extend_from_slice(&dump(e));
                }
                bytes_out.push(b'\n');
                out.write_all(&bytes_out).expect("write");
                checksum = checksum.wrapping_add(bytes_out.len() as u64);
            }
            Err(e) => {
                if parse_only {
                    checksum = checksum.wrapping_add(e.offset as u64).wrapping_mul(31);
                    continue;
                }
                let line = dump_error(&src, &e);
                bytes_out.clear();
                bytes_out.extend_from_slice(id.as_bytes());
                bytes_out.push(b'\t');
                bytes_out.extend_from_slice(&line);
                bytes_out.push(b'\n');
                out.write_all(&bytes_out).expect("write");
                checksum = checksum.wrapping_add(line.len() as u64);
            }
        }
    }
    out.flush().expect("flush");
    eprintln!("{} sources parsed (checksum {:x})", n, checksum);
}
