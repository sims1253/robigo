//! Parse every source in a manifest and dump the parse trees in the
//! oracle's canonical format, one physical line per source:
//! `id \t dump \x01 dump ...` or `id \t ERROR <line> <col> <message>`.
//!
//! Manifest lines are `id \t path \t enc`, with `enc` one of
//! `u` (UTF-8/ASCII, parsed as is) and `l` (Latin-1, transcoded to UTF-8
//! first). Output goes to the file named by the second argument, or stdout
//! for `-`. This is the tool behind `scripts/parse-corpus.py` and the
//! parse-throughput measurements.

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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: parse-batch MANIFEST OUTFILE|-");
        std::process::exit(2);
    }
    let manifest = std::fs::read_to_string(&args[1]).expect("read manifest");
    let stdout = std::io::stdout();
    let file = std::fs::File::create(&args[2]).ok();
    let mut out: Box<dyn Write> = if args[2] == "-" {
        Box::new(BufWriter::new(stdout.lock()))
    } else {
        match file {
            Some(f) => Box::new(BufWriter::new(f)),
            None => Box::new(BufWriter::new(stdout.lock())),
        }
    };
    // A cheap checksum over the trees keeps the optimizer from eliding the
    // work in throughput runs.
    let mut checksum: u64 = 0;
    let mut n = 0u64;
    let mut bytes_out: Vec<u8> = Vec::new();
    for line in manifest.lines() {
        let Some((id, rest)) = line.split_once('\t') else { continue };
        let (path, enc) = match rest.split_once('\t') {
            Some((p, e)) => (p, e),
            None => (rest, "u"),
        };
        let Ok(raw) = std::fs::read(Path::new(path)) else {
            continue;
        };
        let src = if enc == "l" { transcode_latin1(&raw) } else { String::from_utf8_lossy(&raw).into_owned() };
        n += 1;
        match parse_program(&src) {
            Ok(exprs) => {
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
