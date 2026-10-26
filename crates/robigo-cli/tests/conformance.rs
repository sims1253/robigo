//! Differential conformance tests against GNU R.
//!
//! Every `tests/r/**/*.R` script is run with the `robigo` binary and its
//! stdout, stderr and exit status compared byte-for-byte with GNU R's
//! (`*.Rout`, `*.Rerr`, `*.Rstatus`, recorded by `scripts/regen-expected.sh`).
//! A script passes when all three match.
//!
//! Ratchet: `tests/r/passing.txt` lists scripts that must pass. A listed
//! script that fails fails the test; newly passing scripts are reported so
//! they can be added (`ROBIGO_CONF_UPDATE=1` rewrites the file).
//!
//! Environment:
//!   ROBIGO_CONF_FILTER=substr   only run matching scripts
//!   ROBIGO_CONF_DIFF=1          print diffs for failures

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

fn run_rr(script: &Path) -> (Vec<u8>, Vec<u8>, i32, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_robigo"))
        .arg(script)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .current_dir(script.parent().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn robigo");
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
    (t_out.join().unwrap(), t_err.join().unwrap(), status, timed_out)
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
                s.push_str(&format!("   {} robigo: {}\n", mark, g.get(j).copied().unwrap_or("<eof>")));
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
    let read = |ext: &str| std::fs::read(format!("{}.{}", base.display(), ext)).ok();
    let exp_out = read("Rout").unwrap_or_else(|| panic!("no recorded output for {} (run scripts/regen-expected.sh)", rel));
    let exp_err = read("Rerr").unwrap_or_default();
    let exp_status: i32 =
        read("Rstatus").and_then(|s| String::from_utf8_lossy(&s).trim().parse().ok()).unwrap_or(0);
    let (out, err, status, timed_out) = run_rr(script);
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let mut diff = String::new();
    if timed_out {
        diff.push_str("    timed out\n");
    }
    // Bytes, not decoded text: invalid UTF-8 must not compare equal.
    let stdout_ok = out == exp_out && !timed_out;
    if !stdout_ok {
        diff.push_str(&first_diff(&text(&exp_out), &text(&out)));
    }
    let stderr_ok = err == exp_err;
    let status_ok = status == exp_status;
    if stdout_ok && !stderr_ok {
        diff.push_str("    stderr differs:\n");
        diff.push_str(&first_diff(&text(&exp_err), &text(&err)));
    }
    if !stdout_ok && !err.is_empty() && exp_err != err {
        let err = text(&err);
        let tail: Vec<&str> = err.lines().take(3).collect();
        diff.push_str(&format!("    robigo stderr: {}\n", tail.join(" | ")));
    }
    if !status_ok {
        diff.push_str(&format!("    exit status {} (GNU R {})\n", status, exp_status));
    }
    Outcome { rel, stdout_ok, stderr_ok, status_ok, diff }
}

#[test]
fn conformance() {
    let root = root();
    let mut scripts = Vec::new();
    collect(&root.join("tests/r"), &mut scripts);
    scripts.sort();
    let filter = std::env::var("ROBIGO_CONF_FILTER").ok();
    if let Some(f) = &filter {
        scripts.retain(|p| p.display().to_string().contains(f.as_str()));
    }
    let show_diff = std::env::var_os("ROBIGO_CONF_DIFF").is_some();

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
        let passed = r.stdout_ok && r.stderr_ok && r.status_ok;
        if passed {
            pass.insert(r.rel.clone());
            if !listed.contains(&r.rel) {
                new_passes.push(r.rel.clone());
            }
        } else if listed.contains(&r.rel) {
            regressions.push(r);
        }
        let mark = if passed { "ok  " } else if r.stdout_ok { "ok* " } else { "FAIL" };
        eprintln!("{} {}", mark, r.rel);
        if show_diff && !r.diff.is_empty() {
            eprint!("{}", r.diff);
        }
    }
    eprintln!(
        "\nconformance: {}/{} pass; {} stdout, {} stderr, {} exit status match",
        pass.len(),
        results.len(),
        results.iter().filter(|r| r.stdout_ok).count(),
        n_err_ok,
        n_status_ok
    );
    if !new_passes.is_empty() {
        eprintln!("newly passing (not yet in tests/r/passing.txt):");
        for p in &new_passes {
            eprintln!("  {}", p);
        }
    }
    if std::env::var_os("ROBIGO_CONF_UPDATE").is_some() && filter.is_none() {
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

/// The bytecode VM and the JIT must not change behaviour: every
/// conformance script produces identical stdout, stderr and status in the
/// interpreter, the VM without the JIT, and the VM with it.
#[test]
fn compiled_matches_interpreted() {
    let root = root();
    let mut scripts = Vec::new();
    collect(&root.join("tests/r"), &mut scripts);
    scripts.sort();
    let run = |script: &Path, compile: bool, jit: bool| {
        let out = Command::new(env!("CARGO_BIN_EXE_robigo"))
            .arg(script)
            .env("LANGUAGE", "en")
            .env("LC_ALL", "C.UTF-8")
            .env("TZ", "UTC")
            .env("ROBIGO_COMPILE", if compile { "1" } else { "0" })
            .env("ROBIGO_JIT", if jit { "1" } else { "0" })
            .current_dir(script.parent().unwrap())
            .output()
            .expect("spawn robigo");
        (out.stdout, out.stderr, out.status.code())
    };
    let mut bad = Vec::new();
    for s in &scripts {
        let interp = run(s, false, false);
        for (what, compile, jit) in [("VM", true, false), ("VM+JIT", true, true)] {
            if run(s, compile, jit) != interp {
                bad.push(format!("{} ({})", s.strip_prefix(&root).unwrap().display(), what));
            }
        }
    }
    assert!(bad.is_empty(), "compiled and interpreted runs differ for: {:?}", bad);
}

/// Compiled code must stay compiled: the guards that check `+`, `{`,
/// `<-` etc. still mean the base functions succeed on ordinary programs.
/// (A failing guard is still correct, just slow, so the other tests
/// would not notice.)
#[test]
fn vm_guards_hold() {
    let root = root();
    let script = root.join("benchmarks/suite/nbody.R");
    let out = Command::new(env!("CARGO_BIN_EXE_robigo"))
        .arg(&script)
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .env("ROBIGO_COMPILE", "1")
        .env("ROBIGO_VM_TRACE", "1")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .output()
        .expect("spawn robigo");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("guard failed"), "compiled code fell back to the interpreter:\n{}", err);
}
