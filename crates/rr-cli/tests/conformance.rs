//! Differential conformance tests against GNU R.
//!
//! Every `tests/r/**/*.R` script is run with the `rr` binary and its stdout
//! compared byte-for-byte with GNU R's (`*.Rout`, recorded by
//! `scripts/regen-expected.sh`). Stderr (`*.Rerr`) and the exit status are
//! compared too, but reported separately.
//!
//! Ratchet: `tests/r/passing.txt` lists scripts that must pass. A listed
//! script that fails fails the test; newly passing scripts are reported so
//! they can be added (`RR_CONF_UPDATE=1` rewrites the file).
//!
//! Environment:
//!   RR_CONF_FILTER=substr   only run matching scripts
//!   RR_CONF_DIFF=1          print diffs for failures

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "R") {
            out.push(p);
        }
    }
}

struct Outcome {
    rel: String,
    stdout_ok: bool,
    stderr_ok: bool,
    status_ok: bool,
    diff: String,
}

fn run_rr(script: &Path) -> (String, String, i32, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rr"))
        .arg(script)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .current_dir(script.parent().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rr");
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st.code().unwrap_or(-1);
        }
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            timed_out = true;
            break -1;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let so = String::from_utf8_lossy(&t_out.join().unwrap()).into_owned();
    let se = String::from_utf8_lossy(&t_err.join().unwrap()).into_owned();
    (so, se, status, timed_out)
}

fn first_diff(expected: &str, got: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let g: Vec<&str> = got.lines().collect();
    let n = e.len().max(g.len());
    for i in 0..n {
        if e.get(i) != g.get(i) {
            let lo = i.saturating_sub(2);
            let mut s = format!("    first difference at line {}:\n", i + 1);
            for j in lo..(i + 3).min(n) {
                let mark = if j == i { ">" } else { " " };
                s.push_str(&format!("   {} R : {}\n", mark, e.get(j).copied().unwrap_or("<eof>")));
                s.push_str(&format!("   {} rr: {}\n", mark, g.get(j).copied().unwrap_or("<eof>")));
            }
            return s;
        }
    }
    if expected != got {
        return "    differs in trailing newline/whitespace\n".into();
    }
    String::new()
}

fn check(root: &Path, script: &Path) -> Outcome {
    let rel = script.strip_prefix(root).unwrap().display().to_string();
    let base = script.with_extension("");
    let read = |ext: &str| std::fs::read_to_string(format!("{}.{}", base.display(), ext)).ok();
    let exp_out = read("Rout").unwrap_or_else(|| panic!("no recorded output for {} (run scripts/regen-expected.sh)", rel));
    let exp_err = read("Rerr").unwrap_or_default();
    let exp_status: i32 = read("Rstatus").and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let (out, err, status, timed_out) = run_rr(script);
    let mut diff = String::new();
    if timed_out {
        diff.push_str("    timed out\n");
    }
    let stdout_ok = out == exp_out && !timed_out;
    if !stdout_ok {
        diff.push_str(&first_diff(&exp_out, &out));
    }
    let stderr_ok = err == exp_err;
    let status_ok = (status == 0) == (exp_status == 0);
    if stdout_ok && !stderr_ok {
        diff.push_str("    stderr differs:\n");
        diff.push_str(&first_diff(&exp_err, &err));
    }
    if !stdout_ok && !err.is_empty() && exp_err != err {
        let tail: Vec<&str> = err.lines().take(3).collect();
        diff.push_str(&format!("    rr stderr: {}\n", tail.join(" | ")));
    }
    Outcome { rel, stdout_ok, stderr_ok, status_ok, diff }
}

#[test]
fn conformance() {
    let root = root();
    let mut scripts = Vec::new();
    collect(&root.join("tests/r"), &mut scripts);
    scripts.sort();
    let filter = std::env::var("RR_CONF_FILTER").ok();
    if let Some(f) = &filter {
        scripts.retain(|p| p.display().to_string().contains(f.as_str()));
    }
    let show_diff = std::env::var_os("RR_CONF_DIFF").is_some();

    // Run on two worker threads.
    let jobs = std::sync::Mutex::new(scripts.clone());
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| loop {
                let next = jobs.lock().unwrap().pop();
                let Some(p) = next else { break };
                let o = check(&root, &p);
                results.lock().unwrap().push(o);
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.rel.cmp(&b.rel));

    let passing_file = root.join("tests/r/passing.txt");
    let listed: BTreeSet<String> = std::fs::read_to_string(&passing_file)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let mut pass = BTreeSet::new();
    let mut regressions = Vec::new();
    let mut new_passes = Vec::new();
    let (mut n_err_ok, mut n_status_ok) = (0, 0);
    for r in &results {
        if r.stderr_ok {
            n_err_ok += 1;
        }
        if r.status_ok {
            n_status_ok += 1;
        }
        if r.stdout_ok {
            pass.insert(r.rel.clone());
            if !listed.contains(&r.rel) {
                new_passes.push(r.rel.clone());
            }
        } else if listed.contains(&r.rel) {
            regressions.push(r);
        }
        let mark = if r.stdout_ok { if r.stderr_ok { "ok  " } else { "ok* " } } else { "FAIL" };
        eprintln!("{} {}", mark, r.rel);
        if show_diff && !r.diff.is_empty() {
            eprint!("{}", r.diff);
        }
    }
    eprintln!(
        "\nconformance: {}/{} stdout match, {} stderr match, {} exit status match",
        pass.len(),
        results.len(),
        n_err_ok,
        n_status_ok
    );
    if !new_passes.is_empty() {
        eprintln!("newly passing (not yet in tests/r/passing.txt):");
        for p in &new_passes {
            eprintln!("  {}", p);
        }
    }
    if std::env::var_os("RR_CONF_UPDATE").is_some() && filter.is_none() {
        let mut all: BTreeSet<String> = listed.clone();
        all.extend(pass.iter().cloned());
        let body: String = all.iter().map(|s| format!("{}\n", s)).collect();
        std::fs::write(&passing_file, format!("# Conformance ratchet: scripts that must keep matching GNU R.\n{}", body))
            .unwrap();
    }
    if !regressions.is_empty() {
        let mut msg = format!("{} ratcheted script(s) regressed:\n", regressions.len());
        for r in regressions {
            msg.push_str(&format!("  {}\n{}", r.rel, r.diff));
        }
        panic!("{}", msg);
    }
}

/// The bytecode VM must not change behaviour: every conformance script
/// produces identical stdout, stderr and status with RR_COMPILE=1.
#[test]
fn compiled_matches_interpreted() {
    let root = root();
    let mut scripts = Vec::new();
    collect(&root.join("tests/r"), &mut scripts);
    scripts.sort();
    let run = |script: &Path, compile: bool| {
        let out = Command::new(env!("CARGO_BIN_EXE_rr"))
            .arg(script)
            .env("LANGUAGE", "en")
            .env("LC_ALL", "C.UTF-8")
            .env("TZ", "UTC")
            .env("RR_COMPILE", if compile { "1" } else { "0" })
            .current_dir(script.parent().unwrap())
            .output()
            .expect("spawn rr");
        (out.stdout, out.stderr, out.status.code())
    };
    let mut bad = Vec::new();
    for s in &scripts {
        if run(s, false) != run(s, true) {
            bad.push(s.strip_prefix(&root).unwrap().display().to_string());
        }
    }
    assert!(bad.is_empty(), "compiled and interpreted runs differ for: {:?}", bad);
}
