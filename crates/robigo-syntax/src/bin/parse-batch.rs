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
//! `--parse-only` skips the dump and folds a checksum over the trees
//! instead: the same parsing work (values included, so number scanning
//! cannot be elided), no formatting, for throughput runs.

use std::io::{BufWriter, Write};
use std::path::Path;

use robigo_syntax::dump::{dump, dump_error};
use robigo_syntax::{Arena, parse_program};
use robigo_syntax::{Arg, Const, Expr};

fn transcode_latin1(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(b as char); // Latin-1 maps bytes to U+0000..U+00FF
    }
    s
}

/// R's oracle parses files through a text connection, whose newline
/// translation turns CR and CRLF into LF before the parser sees a single
/// byte. Files without a CR (almost all) pass through unmoved.
fn strip_cr(raw: Vec<u8>) -> Vec<u8> {
    if !raw.contains(&b'\r') {
        return raw;
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
/// work in throughput runs: O(nodes/8-byte words) of arithmetic over the
/// parts that matter (spans, constant bits, name and string bytes), no
/// formatting.
fn hash_bytes(h: u64, bytes: &[u8]) -> u64 {
    let mut h = h;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        let mut w = [0u8; 8];
        w.copy_from_slice(c);
        h = h.wrapping_mul(0x100000001b3).wrapping_add(u64::from_le_bytes(w));
    }
    for &b in chunks.remainder() {
        h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
    }
    h
}

fn hash_arg(h: u64, a: &Arg) -> u64 {
    let mut h = h;
    if let Some(n) = &a.name {
        h = hash_bytes(h, n.as_bytes());
        h ^= 0x5a;
    }
    match &a.value {
        Some(v) => hash_expr(h, v),
        None => h ^ 0x2d,
    }
}

fn hash_expr(mut h: u64, e: &Expr) -> u64 {
    h = h.wrapping_mul(0x100000001b3).wrapping_add((e.span().start as u64) << 32 ^ e.span().end as u64);
    match e {
        Expr::Const(c, _) => {
            h ^= 0x63;
            match c {
                Const::Null => h = h.wrapping_mul(3),
                Const::Lgl(None) | Const::Int(None) | Const::DblNa | Const::CplxNa | Const::Str(None) => {
                    h = h.wrapping_mul(5)
                }
                Const::Lgl(Some(b)) => h = h.wrapping_mul(*b as u64 + 7),
                Const::Int(Some(i)) => h = h.wrapping_add(*i as u64),
                Const::Dbl(x) => h = h.wrapping_add(x.to_bits()),
                Const::Cplx(re, im) => h = h.wrapping_add(re.to_bits() ^ im.to_bits().rotate_left(17)),
                Const::Str(Some(s)) => h = hash_bytes(h, s),
            }
        }
        Expr::Sym(name, _) => h = hash_bytes(h ^ 0x73, name.as_bytes()),
        Expr::Placeholder(_) => h ^= 0x70,
        Expr::Call { fun, args, .. } => {
            h ^= 0x28;
            h = hash_expr(h, fun);
            for a in *args {
                h = hash_arg(h, a);
            }
        }
        Expr::Function { formals, body, .. } => {
            h ^= 0x66;
            for f in *formals {
                h = hash_bytes(h, f.name.as_bytes());
                h ^= 0x3d;
                if let Some(d) = &f.default {
                    h = hash_expr(h, d);
                }
            }
            h = hash_expr(h, body);
        }
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let parse_only = args.iter().any(|a| a == "--parse-only");
    let rest: Vec<&str> = args[1..].iter().filter(|a| **a != "--parse-only").map(String::as_str).collect();
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
        // R's oracle parses files through a text connection, whose newline
        // translation turns CR and CRLF into LF before the parser sees a
        // single byte; do the same here so both sides parse one stream.
        let raw = strip_cr(raw);
        // Valid UTF-8 (the common case) becomes a String without a copy;
        // invalid input goes through the lossy conversion as before.
        let src = if enc == "l" {
            transcode_latin1(&raw)
        } else {
            match String::from_utf8(raw) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            }
        };
        n += 1;
        // One arena per source: it frees the whole tree at once.
        let arena = Arena::new();
        match parse_program(&arena, &src) {
            Ok(exprs) => {
                if parse_only {
                    checksum = checksum.wrapping_mul(0x100000001b3).wrapping_add(src.len() as u64);
                    for e in &exprs {
                        checksum = hash_expr(checksum, e);
                    }
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
                    checksum = checksum.wrapping_mul(31).wrapping_add(e.offset as u64);
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
