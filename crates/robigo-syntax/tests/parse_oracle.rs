//! Compare our parse trees with GNU R's, as recorded by
//! `scripts/regen-parse.sh` in `tests/parse/**/*.dump`.

use std::path::{Path, PathBuf};

use robigo_syntax::dump::dump;
use robigo_syntax::parse_program;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn our_dump(src: &str) -> String {
    match parse_program(src) {
        Ok(exprs) => exprs.iter().map(dump).collect::<Vec<_>>().join("\n"),
        Err(_) => "ERROR".to_string(),
    }
}

fn check_dir(dir: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.extension().is_none_or(|e| e != "R") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let expected = std::fs::read_to_string(path.with_extension("dump"))
            .unwrap_or_else(|_| panic!("missing oracle dump for {}", path.display()));
        let ours = our_dump(&src);
        let expected_lines: Vec<&str> = expected.lines().collect();
        let our_lines: Vec<&str> = ours.lines().collect();
        if expected_lines != our_lines {
            let mut msg = format!("{}:\n", path.display());
            for i in 0..expected_lines.len().max(our_lines.len()) {
                let e = expected_lines.get(i).copied().unwrap_or("<none>");
                let o = our_lines.get(i).copied().unwrap_or("<none>");
                if e != o {
                    msg.push_str(&format!("  expr {}:\n    R:  {}\n    robigo: {}\n", i + 1, e, o));
                }
            }
            failures.push(msg);
        }
    }
    failures
}

#[test]
fn parse_trees_match_gnu_r() {
    let root = workspace_root();
    let mut failures = check_dir(&root.join("tests/parse"));
    failures.extend(check_dir(&root.join("tests/parse/errors")));
    if !failures.is_empty() {
        panic!("{} file(s) differ from GNU R:\n{}", failures.len(), failures.join("\n"));
    }
}

/// Files in GNU R's `src/library` whose parse we knowingly do not match.
const KNOWN_DIVERGENCES: &[(&str, &str)] =
    &[("graphics/demo/Hershey.R", "\\xc1 escapes produce a Latin-1 byte; robigo strings are always valid UTF-8")];

/// Parse a large external corpus (e.g. GNU R's own `src/library`) and compare
/// against dumps from `scripts/oracle-parse-corpus.R`. Run with
/// `ROBIGO_PARSE_CORPUS=SRC_DIR ROBIGO_PARSE_DUMPS=DUMP_DIR cargo test -- --ignored`.
#[test]
#[ignore]
fn external_corpus_matches_gnu_r() {
    let (Ok(src), Ok(dumps)) = (std::env::var("ROBIGO_PARSE_CORPUS"), std::env::var("ROBIGO_PARSE_DUMPS")) else {
        panic!("set ROBIGO_PARSE_CORPUS and ROBIGO_PARSE_DUMPS");
    };
    let mut stack = vec![PathBuf::from(&src)];
    let (mut total, mut failed) = (0, Vec::new());
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "R") {
                continue;
            }
            let rel = path.strip_prefix(&src).unwrap();
            if KNOWN_DIVERGENCES.iter().any(|(f, _)| Path::new(f) == rel) {
                continue;
            }
            let dump_path = Path::new(&dumps).join(format!("{}.dump", rel.display()));
            let Ok(expected) = std::fs::read_to_string(&dump_path) else { continue };
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let src_text = String::from_utf8_lossy(&bytes);
            total += 1;
            let ours = our_dump(&src_text);
            let e: Vec<&str> = expected.lines().collect();
            let o: Vec<&str> = ours.lines().collect();
            if e != o {
                let first = (0..e.len().max(o.len())).find(|&i| e.get(i) != o.get(i)).unwrap();
                let clip = |s: Option<&&str>| s.map(|s| s.chars().take(300).collect::<String>()).unwrap_or_default();
                failed.push(format!(
                    "{} (expr {}):\n    R:  {}\n    robigo: {}",
                    rel.display(),
                    first + 1,
                    clip(e.get(first)),
                    clip(o.get(first))
                ));
            }
        }
    }
    failed.sort();
    eprintln!("{} / {} files match", total - failed.len(), total);
    if !failed.is_empty() {
        panic!("{} file(s) differ:\n{}", failed.len(), failed.join("\n"));
    }
}
