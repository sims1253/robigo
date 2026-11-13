//! The isolate holds all mutable runtime state (`docs/g3.md` §11, L6): no
//! `thread_local!` and no `static mut` in the runtime's crates, except the one
//! thread-local the C API exports read, which its line (or the line above)
//! names with the marker `L6: the isolate thread-local`.
//!
//! Scans the library and build sources (`src/**/*.rs`, `build.rs`) of every
//! crate under `crates/` named `robigo-rt*` or `robigo-heap*`, the crates CI's
//! `rt` rule covers. Test code may keep fixture state where it likes.

use std::fs;
use std::path::{Path, PathBuf};

// Built by concatenation so that this file does not match itself.
const THREAD_LOCAL: &str = concat!("thread", "_local!");
const STATIC_MUT: &str = concat!("static", " mut ");
const MARKER: &str = concat!("L6: the isolate", " thread-local");

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn rt_crate_sources() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for e in fs::read_dir(crates).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("robigo-rt") || name.starts_with("robigo-heap")) {
            continue;
        }
        rust_sources(&e.path().join("src"), &mut files);
        let build = e.path().join("build.rs");
        if build.exists() {
            files.push(build);
        }
    }
    files.sort();
    files
}

/// (file:line, what) for every offending line; and the marked thread-locals.
fn scan(files: &[PathBuf]) -> (Vec<String>, Vec<String>) {
    let (mut bad, mut marked) = (Vec::new(), Vec::new());
    for f in files {
        let text = fs::read_to_string(f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap();
            let at = format!("{}:{}", f.display(), i + 1);
            if code.contains(STATIC_MUT) {
                bad.push(format!("{at}: `static mut`"));
            }
            if code.contains(THREAD_LOCAL) {
                let named = line.contains(MARKER) || (i > 0 && lines[i - 1].contains(MARKER));
                if named {
                    marked.push(at);
                } else {
                    bad.push(format!("{at}: `thread_local!`"));
                }
            }
        }
    }
    (bad, marked)
}

#[test]
fn the_runtime_keeps_its_state_in_the_isolate() {
    let files = rt_crate_sources();
    assert!(files.iter().any(|f| f.ends_with("robigo-rt/src/lib.rs")), "found no rt crate sources: {files:?}");
    let (bad, marked) = scan(&files);
    assert!(bad.is_empty(), "mutable state outside the isolate (docs/g3.md §11):\n{}", bad.join("\n"));
    assert!(marked.len() <= 1, "more than one isolate thread-local (L6 allows one):\n{}", marked.join("\n"));
}

#[test]
fn the_scan_finds_what_it_rejects() {
    let dir = std::env::temp_dir().join(format!("robigo-rt-isolate-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a.rs");
    let src = format!(
        "{THREAD_LOCAL} {{ static A: u8 = 0; }}\npub {STATIC_MUT}B: u8 = 0;\n\
         // {THREAD_LOCAL} in a comment is fine\n// {MARKER}\n{THREAD_LOCAL} {{ static C: u8 = 0; }}\n"
    );
    fs::write(&f, src).unwrap();
    let (bad, marked) = scan(std::slice::from_ref(&f));
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad[0].ends_with(":1: `thread_local!`"), "{bad:?}");
    assert!(bad[1].ends_with(":2: `static mut`"), "{bad:?}");
    assert_eq!(marked.len(), 1, "{marked:?}");
    assert!(marked[0].ends_with(":5"), "{marked:?}");
}
