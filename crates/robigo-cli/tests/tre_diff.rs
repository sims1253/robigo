//! The TRE routing rule against GNU R's TRE (issue #423).
//!
//! robigo matches most TRE patterns with its own engine and sends what that
//! engine can get wrong to GNU R's TRE (`builtins/grep.rs`,
//! `tre_needs_native`). With `ROBIGO_TRE_DIFF=<file>`, every pattern kept on
//! the fast engine also runs through TRE, and each disagreement is logged.
//! This test runs, under that log:
//!
//! * a generated set of patterns around each routed construct (greedy
//!   repetitions, alternations, groups, anchors, classes, their
//!   combinations) through every regex builtin, on ASCII and non-ASCII
//!   subjects;
//! * every `tests/r` script that uses a regex builtin;
//! * with `ROBIGO_TRE_DIFF_CORPORA=1`, every corpus case that does
//!   (`tests/corpora`: fastr's snippets, and the `cases/*.R` scripts).
//!
//! Any disagreement on a pattern the rule keeps on the fast engine fails it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn robigo_bin() -> PathBuf {
    std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), PathBuf::from)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

const REGEX_FNS: &[&str] = &["grep", "sub(", "regexpr", "regexec", "strsplit", "regmatches", "list.files"];

fn uses_regex(code: &str) -> bool {
    REGEX_FNS.iter().any(|f| code.contains(f))
}

/// Run `script` (in `dir`) with the disagreement log at `log`; its own
/// output and status do not matter here.
fn run(script: &Path, dir: &Path, log: &Path, limit: Duration) {
    let mut child = Command::new(robigo_bin())
        .arg(script)
        .current_dir(dir)
        .env("ROBIGO_TRE_DIFF", log)
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("run robigo");
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn r_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Patterns around the routed constructs that stay on the fast engine.
fn generated_patterns() -> Vec<String> {
    let atoms = [
        "a",
        "b",
        "ab",
        "[ab]",
        "[[:alpha:]]",
        ".",
        "(a)",
        "(a|b)",
        "(ab|a)",
        "(a|ab)",
        "x",
        "é",
        "[[:space:]]",
        "(a*)",
        "[^a]",
    ];
    let quants = ["", "*", "+", "?", "{1,2}", "{0,1}", "{2}"];
    let pieces: Vec<String> = atoms.iter().flat_map(|a| quants.iter().map(move |q| format!("{a}{q}"))).collect();
    let mut pats: Vec<String> = pieces.clone();
    for (i, p) in pieces.iter().enumerate() {
        for (j, q) in pieces.iter().enumerate() {
            // every concatenation, and a stride of the alternations
            pats.push(format!("{p}{q}"));
            if (i * pieces.len() + j).is_multiple_of(3) {
                pats.push(format!("{p}|{q}"));
                pats.push(format!("({p}|{q})"));
            }
            if (i + j).is_multiple_of(11) {
                pats.push(format!("^{p}{q}"));
                pats.push(format!("{p}{q}$"));
                pats.push(format!("({p})({q})"));
                pats.push(format!("({p}{q})+"));
            }
        }
    }
    pats
}

fn generated_script(pats: &[String]) -> String {
    let subjects = [
        "", "a", "b", "ab", "aab", "abab", "ba", "xaxbx", "aaa bbb", "a b\tab", "éa é", "ébé", "abé", "bbbaaab", "x",
        "abcabc",
    ];
    let mut s = String::from("x <- c(");
    s.push_str(&subjects.iter().map(|x| r_str(x)).collect::<Vec<_>>().join(", "));
    s.push_str(")\nxa <- x[!grepl(\"[^ -~]\", x, perl = TRUE)]\n");
    s.push_str("f <- function(p) for (v in list(x, xa)) {\n");
    for call in [
        "regexpr(p, v)",
        "gregexpr(p, v)",
        "regexec(p, v)",
        "sub(p, \"<\\\\1>\", v)",
        "gsub(p, \"<\\\\0>\", v)",
        "gsub(p, \"-\", v)",
        "strsplit(v, p)",
        "grepl(p, v)",
        "regexpr(p, v, ignore.case = TRUE)",
        "regexpr(p, v, useBytes = TRUE)",
    ] {
        s.push_str(&format!("  try({call}, silent = TRUE)\n"));
    }
    s.push_str("}\n");
    for p in pats {
        s.push_str(&format!("f({})\n", r_str(p)));
    }
    s
}

fn fresh_log(dir: &Path, name: &str) -> PathBuf {
    let log = dir.join(name);
    let _ = std::fs::remove_file(&log);
    log
}

fn check(log: &Path, what: &str) {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    if !lines.is_empty() {
        let shown: Vec<&str> = lines.iter().take(20).copied().collect();
        panic!(
            "{}: {} disagreement(s) between the fast engine and GNU R's TRE on patterns the rule keeps on the \
             fast engine:\n{}",
            what,
            lines.len(),
            shown.join("\n")
        );
    }
}

#[test]
fn tre_diff_generated() {
    let dir = std::env::temp_dir().join(format!("robigo-tre-diff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pats = generated_patterns();
    assert!(pats.len() > 3000);
    let script = dir.join("generated.R");
    std::fs::write(&script, generated_script(&pats)).unwrap();
    let log = fresh_log(&dir, "generated.log");
    run(&script, &dir, &log, Duration::from_secs(600));
    check(&log, "generated patterns");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tre_diff_conformance_scripts() {
    let root = root();
    let dir = std::env::temp_dir().join(format!("robigo-tre-diff-conf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = fresh_log(&dir, "conformance.log");
    let mut scripts = Vec::new();
    collect(&root.join("tests/r"), &mut scripts);
    let mut n = 0;
    for s in scripts {
        if uses_regex(&std::fs::read_to_string(&s).unwrap_or_default()) {
            run(&s, s.parent().unwrap(), &log, Duration::from_secs(60));
            n += 1;
        }
    }
    assert!(n > 10, "only {n} scripts use regexes");
    eprintln!("tre_diff: {n} tests/r scripts use a regex builtin");
    check(&log, "tests/r scripts");
    if std::env::var_os("ROBIGO_TRE_DIFF_CORPORA").is_some() {
        corpora(&root, &dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<PathBuf> = rd.map(|e| e.unwrap().path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "R") {
            out.push(p);
        }
    }
}

/// The corpora's cases that use a regex builtin: fastr's snippets in
/// batches (each in `try`), the scripts one by one.
fn corpora(root: &Path, dir: &Path) {
    let log = fresh_log(dir, "corpora.log");
    let cases = std::fs::read_to_string(root.join("tests/corpora/fastr/cases.jsonl")).unwrap_or_default();
    let snippets: Vec<String> = cases
        .lines()
        .filter_map(|l| {
            let i = l.find("\"code\": ")? + 8;
            let rest = &l[i..];
            let end = rest.find("\", \"tests\"")?;
            Some(rest[..=end].to_string())
        })
        .filter(|c| uses_regex(c))
        .collect();
    for (k, batch) in snippets.chunks(100).enumerate() {
        let mut s = String::new();
        for code in batch {
            // `code` is a JSON string literal, which R parses the same way
            s.push_str(&format!("try(eval(parse(text = {code})), silent = TRUE)\n"));
        }
        let script = dir.join(format!("fastr-{k}.R"));
        std::fs::write(&script, s).unwrap();
        run(&script, dir, &log, Duration::from_secs(300));
    }
    let mut scripts = Vec::new();
    collect(&root.join("tests/corpora"), &mut scripts);
    let mut n = 0;
    for s in scripts {
        if uses_regex(&std::fs::read_to_string(&s).unwrap_or_default()) {
            run(&s, s.parent().unwrap(), &log, Duration::from_secs(120));
            n += 1;
        }
    }
    eprintln!("tre_diff: {} fastr snippets and {} corpus scripts use a regex builtin", snippets.len(), n);
    assert!(!snippets.is_empty() && n > 0, "no corpus cases found");
    check(&log, "corpora");
}
