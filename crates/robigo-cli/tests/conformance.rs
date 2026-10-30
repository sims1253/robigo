//! Differential conformance tests against GNU R.
//!
//! Every `tests/r/**/*.R` script is run with the `robigo` binary and its
//! stdout, stderr and exit status compared byte-for-byte with GNU R's
//! (`*.Rout`, `*.Rerr`, `*.Rstatus`, recorded by `scripts/regen-expected.sh`).
//! A script passes when all three match.
//!
//! Ratchet: `tests/r/passing/<script>.pass` marks a script that must pass
//! (one empty marker file per script, so two branches that ratchet different
//! scripts add different files and never conflict). A marked script that
//! fails fails the test; newly passing scripts are reported so they can be
//! marked (`ROBIGO_CONF_UPDATE=1` creates their markers).
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

/// The ratchet: one empty marker file per script that must keep passing.
/// A marker is named after the script it ratchets, relative to `tests/r`,
/// plus `.pass`: `tests/r/passing/strings/escape_origin.R.pass`. Adding a
/// case is adding a file, so parallel branches never edit the same path.
const RATCHET_DIR: &str = "tests/r/passing";
const RATCHET_PREFIX: &str = "tests/r";
const RATCHET_SUFFIX: &str = ".pass";

fn ratchet_markers(dir: &Path, rel: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("read {}: {}", dir.display(), e))?.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let rel = rel.join(&name);
        if path.is_dir() {
            ratchet_markers(&path, &rel, out)?;
        } else {
            let Some(script) = name.strip_suffix(RATCHET_SUFFIX) else {
                return Err(format!(
                    "{}: not a ratchet marker (markers are named <script>{})",
                    path.display(),
                    RATCHET_SUFFIX
                ));
            };
            // Ids are repository-relative, as the old list's lines were.
            out.insert(format!("{}/{}", RATCHET_PREFIX, rel.with_file_name(script).display()));
        }
    }
    Ok(())
}

/// The scripts the ratchet requires, from the marker files. The single list
/// this replaces (`tests/r/passing.txt`) is still read while branches
/// migrate (`scripts/ratchet.py migrate`). A marker for a script that does
/// not exist is an error: it used to be a line that silently required
/// nothing.
fn load_ratchet(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    let dir = root.join(RATCHET_DIR);
    if dir.is_dir() {
        ratchet_markers(&dir, Path::new(""), &mut ids)?;
    }
    let legacy = root.join("tests/r/passing.txt");
    match std::fs::read_to_string(&legacy) {
        Ok(text) => {
            ids.extend(text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("read {}: {}", legacy.display(), e)),
    }
    for id in &ids {
        let script = root.join(id);
        if !script.is_file() {
            return Err(format!(
                "ratchet entry {:?} names a script that does not exist ({}); remove the marker",
                id,
                script.display()
            ));
        }
    }
    Ok(ids)
}

struct Outcome {
    rel: String,
    stdout_ok: bool,
    stderr_ok: bool,
    status_ok: bool,
    diff: String,
}

// A .tzdir sidecar contains a directory relative to the script, also used
// by scripts/regen-expected.sh. Resolve before spawning: working directories
// differ between the oracle recorder and the conformance runner.
fn script_command(script: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_robigo"));
    // As scripts/regen-expected.sh records GNU R's output: base only.
    command.env("R_DEFAULT_PACKAGES", "NULL");
    command.env_remove("TZDIR");
    let sidecar = script.with_extension("tzdir");
    match std::fs::read_to_string(&sidecar) {
        Ok(dir) => {
            let path = script.parent().unwrap().join(dir.trim()).canonicalize().expect("resolve .tzdir");
            assert!(path.is_dir(), ".tzdir must name a directory");
            command.env("TZDIR", path);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("read {}: {}", sidecar.display(), err),
    }
    command
}

fn run_rr(script: &Path) -> (Vec<u8>, Vec<u8>, i32, bool) {
    let mut child = script_command(script)
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
    let exp_out =
        read("Rout").unwrap_or_else(|| panic!("no recorded output for {} (run scripts/regen-expected.sh)", rel));
    let exp_err = read("Rerr").unwrap_or_default();
    let exp_status: i32 = read("Rstatus").and_then(|s| String::from_utf8_lossy(&s).trim().parse().ok()).unwrap_or(0);
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
            s.spawn(|| {
                loop {
                    let next = jobs.lock().unwrap().pop();
                    let Some(p) = next else { break };
                    let o = check(&root, &p);
                    results.lock().unwrap().push(o);
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.rel.cmp(&b.rel));

    let listed = load_ratchet(&root).unwrap_or_else(|e| panic!("{}", e));

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
        let mark = if passed {
            "ok  "
        } else if r.stdout_ok {
            "ok* "
        } else {
            "FAIL"
        };
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
        eprintln!("newly passing (not yet ratcheted):");
        for p in &new_passes {
            eprintln!("  {}", p);
        }
    }
    if std::env::var_os("ROBIGO_CONF_UPDATE").is_some() {
        // Only add markers: no existing file is rewritten, so a filtered
        // run can ratchet the cases it ran, and two branches never touch
        // the same marker.
        let mut added = 0;
        for rel in &pass {
            let rel = rel.strip_prefix("tests/r/").unwrap_or(rel);
            let marker = root.join(RATCHET_DIR).join(format!("{}{}", rel, RATCHET_SUFFIX));
            if marker.exists() {
                continue;
            }
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, "").unwrap();
            added += 1;
        }
        if added > 0 {
            eprintln!("\nratcheted {} new script(s) under {}", added, RATCHET_DIR);
        }
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
        let out = script_command(script)
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

/// A fixture tree with one script and an empty ratchet directory.
fn ratchet_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("robigo-ratchet-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tests/r/passing")).unwrap();
    std::fs::create_dir_all(dir.join("tests/r/basics")).unwrap();
    std::fs::write(dir.join("tests/r/basics/arith.R"), "").unwrap();
    dir
}

fn mark(root: &Path, script: &str) {
    let marker = root.join(RATCHET_DIR).join(format!("{}{}", script, RATCHET_SUFFIX));
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(marker, "").unwrap();
}

#[test]
fn ratchet_reads_markers() {
    let root = ratchet_fixture("read");
    mark(&root, "basics/arith.R");
    let listed = load_ratchet(&root).unwrap();
    assert_eq!(listed, ["tests/r/basics/arith.R".to_string()].into_iter().collect());
    std::fs::remove_dir_all(&root).unwrap();
}

/// The failure mode of the old single list: an entry naming no script did
/// nothing, silently. A marker must fail loudly instead.
#[test]
fn ratchet_rejects_marker_for_missing_script() {
    let root = ratchet_fixture("dangling");
    mark(&root, "basics/ghost.R");
    let err = load_ratchet(&root).expect_err("a dangling marker must be an error");
    assert!(err.contains("does not exist") && err.contains("basics/ghost.R"), "{}", err);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn ratchet_rejects_stray_file() {
    let root = ratchet_fixture("stray");
    std::fs::write(root.join(RATCHET_DIR).join("notes.txt"), "not a marker").unwrap();
    let err = load_ratchet(&root).expect_err("a stray file in the ratchet must be an error");
    assert!(err.contains("not a ratchet marker"), "{}", err);
    std::fs::remove_dir_all(&root).unwrap();
}

/// Branches based on the old layout still work: the single list is read too
/// until `scripts/ratchet.py migrate` converts it.
#[test]
fn ratchet_reads_legacy_list() {
    let root = ratchet_fixture("legacy");
    mark(&root, "basics/arith.R");
    std::fs::write(root.join("tests/r/passing.txt"), "# comment\n\ntests/r/basics/arith.R\n").unwrap();
    assert_eq!(load_ratchet(&root).unwrap().len(), 1);
    std::fs::write(root.join("tests/r/passing.txt"), "tests/r/basics/ghost.R\n").unwrap();
    assert!(load_ratchet(&root).is_err(), "a dangling legacy entry must be an error");
    std::fs::remove_dir_all(&root).unwrap();
}
