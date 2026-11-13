//! The isolate holds all mutable runtime state (`docs/g3.md` §11, L6): no
//! `thread_local!` and no `static mut` in the runtime's crates, except the one
//! thread-local the C API exports read. That one carries the marker
//! `L6: the isolate thread-local` in a comment anywhere from the line above
//! the `thread_local!` to the macro's closing delimiter.
//!
//! Scans the library and build sources (`src/**/*.rs`, `build.rs`) of every
//! crate under `crates/` whose directory matches CI's `rt` rule
//! (`crates/robigo-rt*`, `crates/robigo-heap*`, scripts/ci-selection.toml).
//! Test code may keep fixture state where it likes. Comments, string and
//! character literals are blanked before matching, so neither a `//` inside
//! a string nor a mention of the macro in a message changes the verdict.

use std::fs;
use std::path::{Path, PathBuf};

// Built by concatenation so that this file does not match itself.
const MARKER: &str = concat!("L6: the isolate", " thread-local");

/// The crates that exist today; each must be scanned (a rename that moves
/// one out of the scan fails here instead of shrinking it silently).
const KNOWN: &[&str] = &["robigo-rt", "robigo-rtapi", "robigo-rtbuiltins", "robigo-rtcore"];

/// The directory prefixes of CI's `rt` rule (`crates/<prefix>*/**` in
/// scripts/ci-selection.toml); `ci_selection_names_the_same_crates` keeps
/// the two in step.
const RT_PREFIXES: &[&str] = &["robigo-rt", "robigo-heap"];

fn is_rt_crate(dir: &str) -> bool {
    RT_PREFIXES.iter().any(|p| dir.starts_with(p))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for e in entries {
        let e = e.unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let p = e.path();
        // file_type does not follow symlinks: a linked directory is not
        // walked (no cycles); a linked .rs file is still read.
        let ty = e.file_type().unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        if ty.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// (crate directory name, its source files) for every rt crate.
fn rt_crate_sources() -> Vec<(String, Vec<PathBuf>)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut out = Vec::new();
    for e in fs::read_dir(crates).unwrap_or_else(|e| panic!("{}: {e}", crates.display())) {
        let e = e.unwrap_or_else(|e| panic!("{}: {e}", crates.display()));
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_rt_crate(&name) {
            continue;
        }
        let mut files = Vec::new();
        let src = e.path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut files);
        }
        let build = e.path().join("build.rs");
        if build.exists() {
            files.push(build);
        }
        files.sort();
        out.push((name, files));
    }
    out.sort();
    out
}

/// `src` with string and character literals, and unless `keep_comments`
/// comments too, replaced by spaces; newlines are kept, so byte offsets map
/// to the same lines.
fn blank_non_code(src: &str, keep_comments: bool) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let ident = |c: u8| c == b'_' || c.is_ascii_alphanumeric();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                if !keep_comments {
                    blank(&mut out, start, i);
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                // block comments nest in Rust
                let mut depth = 0;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                if !keep_comments {
                    blank(&mut out, start, i);
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                blank(&mut out, start, i);
            }
            // raw strings: r"..", r#".."#, br#".."#, cr#".."# (not raw identifiers r#name)
            b'r' if (i == 0 || !ident(b[i - 1]) || matches!(b[i - 1], b'b' | b'c')) && {
                let mut j = i + 1;
                while b.get(j) == Some(&b'#') {
                    j += 1;
                }
                b.get(j) == Some(&b'"')
            } =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while b[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
                j += 1;
                while j < b.len() && !b[j..].starts_with(&close) {
                    j += 1;
                }
                i = (j + close.len()).min(b.len());
                blank(&mut out, start, i);
            }
            // character literals; a lifetime ('a) has no closing quote
            b'\'' => {
                if b.get(i + 1) == Some(&b'\\') {
                    i += 2;
                    while i < b.len() && b[i] != b'\'' {
                        i += 1;
                    }
                    i = (i + 1).min(b.len());
                    blank(&mut out, start, i);
                } else {
                    let ch = src[i + 1..].chars().next().map_or(0, char::len_utf8);
                    if ch > 0 && b.get(i + 1 + ch) == Some(&b'\'') {
                        i += 2 + ch;
                        blank(&mut out, start, i);
                    } else {
                        i += 1;
                    }
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("only ASCII bytes were written")
}

/// The identifier tokens of blanked code, with their byte offsets.
fn idents(code: &str) -> Vec<(usize, &str)> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_' || b[i].is_ascii_alphabetic() {
            let s = i;
            while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            out.push((s, &code[s..i]));
        } else {
            i += 1;
        }
    }
    out
}

/// The offset just past the delimiter that closes the one opening at or
/// after `from` (`{`, `(` or `[`), or the end of `code`.
fn macro_end(code: &str, from: usize) -> usize {
    let b = code.as_bytes();
    let mut depth = 0usize;
    for (k, &c) in b.iter().enumerate().skip(from) {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    b.len()
}

/// (offending sites, marked thread-locals) of one file, as `label:line`.
fn scan_text(label: &str, src: &str) -> (Vec<String>, Vec<String>) {
    let code = blank_non_code(src, false);
    // the marker must be in a comment: search with literals blanked, comments kept
    let commented = blank_non_code(src, true);
    let line_of = |off: usize| code[..off].bytes().filter(|&c| c == b'\n').count();
    let lines: Vec<&str> = commented.lines().collect();
    let toks = idents(&code);
    let (mut bad, mut marked) = (Vec::new(), Vec::new());
    for (k, &(off, tok)) in toks.iter().enumerate() {
        let line = line_of(off);
        let at = format!("{label}:{}", line + 1);
        let after = code[off + tok.len()..].trim_start();
        if tok == "static"
            && toks.get(k + 1).is_some_and(|&(o, t)| t == "mut" && code[off + tok.len()..o].trim().is_empty())
        {
            bad.push(format!("{at}: `static mut`"));
        }
        if tok == "thread_local" && after.starts_with('!') {
            let end = line_of(macro_end(&code, off));
            let first = line.saturating_sub(1);
            let named = lines[first..=end.min(lines.len() - 1)].iter().any(|l| l.contains(MARKER));
            if named {
                marked.push(at);
            } else {
                bad.push(format!("{at}: `thread_local!`"));
            }
        }
    }
    (bad, marked)
}

#[test]
fn the_runtime_keeps_its_state_in_the_isolate() {
    let crates = rt_crate_sources();
    for name in KNOWN {
        let files = crates.iter().find(|(n, _)| n == name).map(|(_, f)| f);
        assert!(
            files.is_some_and(|f| f.iter().any(|p| p.ends_with("src/lib.rs"))),
            "crate {name} is not scanned, or has no src/lib.rs (scanned: {:?})",
            crates.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }
    let (mut bad, mut marked) = (Vec::new(), Vec::new());
    for f in crates.iter().flat_map(|(_, f)| f) {
        let src = fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let (b, m) = scan_text(&f.display().to_string(), &src);
        bad.extend(b);
        marked.extend(m);
    }
    assert!(bad.is_empty(), "mutable state outside the isolate (docs/g3.md §11):\n{}", bad.join("\n"));
    assert!(marked.len() <= 1, "more than one isolate thread-local (L6 allows one):\n{}", marked.join("\n"));
}

#[test]
fn the_scan_finds_what_it_rejects() {
    let tl = concat!("thread", "_local!");
    let sm = concat!("static", " mut");
    let src = [
        format!("{tl} {{ static A: u8 = 0; }}"),                   // 1: bad
        format!("pub {sm} B: u8 = 0;"),                            // 2: bad
        "static\t/* x */ mut C: u8 = 0;".to_string(),              // 3: bad, odd spacing
        format!("// {tl} and {sm} in a comment"),                  // 4: fine
        format!("const U: &str = \"https://x\"; {sm} D: u8 = 0;"), // 5: bad after an in-string //
        format!("const M: &str = \"{tl} {sm}\";"),                 // 6: fine, a string
        format!("const R: &str = r#\"{tl} \"quoted\" {sm}\"#;"),   // 7: fine, a raw string
        format!("fn f<'a>(x: &'a u8) -> char {{ '\"' }} /* {tl} /* nested */ {sm} */"), // 8: fine
        format!("{tl} {{"),                                        // 9: marked (marker inside)
        format!("    // {MARKER}"),                                // 10
        "    static E: u8 = 0;".to_string(),                       // 11
        "}".to_string(),                                           // 12
        format!("// {MARKER}"),                                    // 13
        format!("std::thread::{tl}(static F: u8 = 0);"),           // 14: marked (line above)
        "let r#static = 1; let mutable = r#static;".to_string(),   // 15: fine
        format!("const S: &str = \"{MARKER}\";"),                  // 16: a marker in a string ...
        format!("{tl}(static G: u8 = 0);"),                        // 17: ... names nothing: bad
        format!("{tl} {{ static H: &str = \"{MARKER}\"; }}"),      // 18: bad, the same inside
    ]
    .join("\n");
    let (bad, marked) = scan_text("a.rs", &src);
    assert_eq!(
        bad,
        [
            "a.rs:1: `thread_local!`",
            "a.rs:2: `static mut`",
            "a.rs:3: `static mut`",
            "a.rs:5: `static mut`",
            "a.rs:17: `thread_local!`",
            "a.rs:18: `thread_local!`",
        ],
        "{bad:#?}"
    );
    assert_eq!(marked, ["a.rs:9", "a.rs:14"], "{marked:#?}");
}

/// `is_rt_crate` mirrors the crate globs of CI's `rt` rule by hand: fail when
/// scripts/ci-selection.toml names crates this scan would not, or the other
/// way round.
#[test]
fn ci_selection_names_the_same_crates() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/ci-selection.toml");
    let toml = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // the `rt` rule runs from its `name = "rt"` line to the next rule
    let start = toml.find("name = \"rt\"").unwrap_or_else(|| panic!("{}: no rule named \"rt\"", path.display()));
    let end = toml[start..].find("[[rules]]").map_or(toml.len(), |k| start + k);
    let mut globs: Vec<&str> =
        toml[start..end].split('"').skip(1).step_by(2).filter(|s| s.starts_with("crates/")).collect();
    globs.sort();
    let mut want: Vec<String> = RT_PREFIXES.iter().map(|p| format!("crates/{p}*/**")).collect();
    want.sort();
    assert_eq!(globs, want, "{}: the rt rule's crate globs differ from RT_PREFIXES", path.display());
}
