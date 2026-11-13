//! Differential conformance against GNU R, for both runtimes.
//!
//! A case is an R script. It runs under a runtime's binary and passes when
//! its stdout, stderr and exit status equal, byte for byte, what GNU R
//! recorded for it (`scripts/regen-expected.sh`). Which recording is used
//! depends on the GNU R version (docs/verification.md, "Oracles"):
//! 4.5.0's sit beside each script, every other version's mirror the suite
//! under `<suite>/R-<version>/`.
//!
//! A [`Column`] is one runtime's view of the cases: the GNU R version it is
//! gated on, the suites it runs and its ratchet (docs/g3.md L1, §14.3). The
//! old runtime ([`OLD`]) is gated on 4.5.0 and ratchets `tests/r` into
//! `tests/r/passing/`; G3's runtime ([`RT`]) is gated on 4.6.2 and ratchets
//! into `passing-rt/` beside each suite's `passing/`. The two columns never
//! touch the same marker.
//!
//! The ratchet is one empty marker file per case,
//! `<suite>/<ratchet>/<script relative to the suite>.pass`, so two branches
//! that ratchet different cases add different files. A marked case that
//! fails fails the run. Markers are only ever added, by `ROBIGO_CONF_UPDATE`
//! in a gated run, and never removed by the runner.
//!
//! Environment ([`Options::from_env`]):
//!   ROBIGO_CONF_FILTER=substr   only run cases whose path contains it
//!   ROBIGO_CONF_DIFF=1          print diffs for failures
//!   ROBIGO_CONF_UPDATE=1        add markers for passing cases (gated runs)
//!   ROBIGO_CONF_SWEEP=1         also run the suites a column runs only for
//!                               its marked cases (`RT`'s `tests/r`), to see
//!                               which pass
//!   ROBIGO_R_VERSION=4.6.2      compare with that GNU R version's
//!                               recordings (default: the column's gated
//!                               version); a run against another version
//!                               only reports: it checks no marker and
//!                               refuses ROBIGO_CONF_UPDATE
//! The binary and the results file are the caller's (each test names its
//! own variables), so one `cargo test --workspace` can run both columns.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// Rebuild when another checkout built this crate last (.cargo/config.toml).
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

/// The GNU R version the old runtime is gated on. Its recordings sit beside
/// each script; every other version's under `<suite>/R-<version>/`. Keep in
/// step with `GATED_R_VERSION` in scripts/corpus.py and `gated` in
/// scripts/regen-expected.sh.
pub const GATED_R_VERSION: &str = "4.5.0";

/// The GNU R version `robigo-rt` is gated on (docs/g3.md L3). Keep in step
/// with `rt` in scripts/regen-expected.sh, the only version that records
/// `tests/rt`.
pub const RT_R_VERSION: &str = "4.6.2";

/// Which of a suite's cases a column runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cases {
    /// Every case: failures are reported, passes can be ratcheted.
    All,
    /// Only the cases the column's ratchet marks, so the ratchet holds
    /// without running a suite the runtime mostly fails; `ROBIGO_CONF_SWEEP`
    /// runs the rest.
    Marked,
}

/// A directory of cases, relative to the repository.
#[derive(Debug)]
pub struct Suite {
    pub dir: &'static str,
    pub cases: Cases,
}

/// One runtime's ratchet column.
#[derive(Debug)]
pub struct Column {
    /// The binary's name, for messages.
    pub runtime: &'static str,
    /// The GNU R version whose recordings the ratchet holds.
    pub gated: &'static str,
    /// The ratchet's directory name inside each suite.
    pub ratchet: &'static str,
    pub suites: &'static [Suite],
    /// The `corpus` of the results file (`Options::results`).
    pub corpus: &'static str,
    /// A single-list ratchet still read beside the markers (repository
    /// relative), while branches migrate (`scripts/ratchet.py migrate`).
    pub legacy_list: Option<&'static str>,
}

/// The old runtime, `robigo`: `tests/r` against GNU R 4.5.0.
pub const OLD: Column = Column {
    runtime: "robigo",
    gated: GATED_R_VERSION,
    ratchet: "passing",
    suites: &[Suite { dir: "tests/r", cases: Cases::All }],
    corpus: "conformance",
    legacy_list: Some("tests/r/passing.txt"),
};

/// G3's runtime, `robigo-rt`: its own cases in `tests/rt`, and the cases
/// of `tests/r` it has ratcheted, against GNU R 4.6.2.
pub const RT: Column = Column {
    runtime: "robigo-rt",
    gated: RT_R_VERSION,
    ratchet: "passing-rt",
    suites: &[Suite { dir: "tests/rt", cases: Cases::All }, Suite { dir: "tests/r", cases: Cases::Marked }],
    corpus: "conformance-rt",
    legacy_list: None,
};

/// The feature requirements a `.requires` sidecar may name.
pub const KNOWN_FEATURES: &[&str] = &["bytecode-fidelity"];

/// How long one case may run.
const TIMEOUT: Duration = Duration::from_secs(20);

/// One run's settings.
#[derive(Debug, Clone)]
pub struct Options {
    /// The binary under test.
    pub bin: PathBuf,
    /// The GNU R version whose recordings to compare with.
    pub version: String,
    pub filter: Option<String>,
    pub diff: bool,
    pub update: bool,
    pub sweep: bool,
    /// The features of `KNOWN_FEATURES` the build under test has.
    pub features: Vec<&'static str>,
    /// Where an unfiltered run writes its results, in scripts/corpus.py's
    /// result format.
    pub results: Option<PathBuf>,
    pub jobs: usize,
}

impl Options {
    /// The settings from the environment (the module documentation lists
    /// the variables), for `column` and `bin`.
    pub fn from_env(column: &Column, bin: PathBuf) -> Result<Options, String> {
        let flag = |name: &str| std::env::var_os(name).is_some();
        let filter = match std::env::var_os("ROBIGO_CONF_FILTER") {
            None => None,
            Some(f) => Some(f.into_string().map_err(|f| format!("ROBIGO_CONF_FILTER={f:?} is not UTF-8"))?),
        };
        Ok(Options {
            bin,
            version: r_version(std::env::var_os("ROBIGO_R_VERSION").as_deref(), column)?,
            filter,
            diff: flag("ROBIGO_CONF_DIFF"),
            update: flag("ROBIGO_CONF_UPDATE"),
            sweep: flag("ROBIGO_CONF_SWEEP"),
            features: Vec::new(),
            results: None,
            jobs: 2,
        })
    }
}

/// The GNU R version from `ROBIGO_R_VERSION`'s value, or the column's gated
/// version when unset. A value that is set but not a version is an error,
/// never the gated version: a run that silently checked the ratchet would
/// claim a comparison that was not asked for.
pub fn r_version(value: Option<&OsStr>, column: &Column) -> Result<String, String> {
    let Some(v) = value else { return Ok(column.gated.to_string()) };
    let v = v.to_str().ok_or_else(|| format!("ROBIGO_R_VERSION={v:?} is not UTF-8"))?;
    if v.is_empty() || !v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return Err(format!("ROBIGO_R_VERSION={v:?} is not a version like 4.6.2"));
    }
    Ok(v.to_string())
}

/// Where `script`'s recording for GNU R `version` lives, without extension.
pub fn expected_base(root: &Path, suite: &str, script: &Path, version: &str) -> PathBuf {
    if version == GATED_R_VERSION {
        return script.with_extension("");
    }
    let dir = root.join(suite);
    let rel = script.strip_prefix(&dir).expect("a script inside its suite");
    dir.join(format!("R-{version}")).join(rel).with_extension("")
}

/// Whether the build has every feature `script`'s `.requires` sidecar
/// names. Requirements select the build that owns a fixture without
/// changing its oracle or removing its ratchet marker.
pub fn enabled(script: &Path, features: &[&str]) -> bool {
    match std::fs::read_to_string(script.with_extension("requires")) {
        Ok(text) => text.split_whitespace().all(|f| {
            assert!(KNOWN_FEATURES.contains(&f), "unknown conformance feature requirement: {f}");
            features.contains(&f)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => panic!("read feature requirements: {e}"),
    }
}

/// Every enabled `*.R` under `dir`, sorted.
pub fn scripts(dir: &Path, features: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "R") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.retain(|p| enabled(p, features));
    out.sort();
    out
}

/// The command that runs `script` with `bin` as GNU R was recorded: base
/// packages only, `LANGUAGE=en`, `C.UTF-8`, UTC, in the script's directory.
/// A `.tzdir` sidecar names a directory relative to the script, which also
/// scripts/regen-expected.sh resolves before setting `TZDIR`; without one,
/// an inherited `TZDIR` is cleared.
pub fn command(bin: &Path, script: &Path) -> Command {
    let mut command = Command::new(bin);
    command.arg(script);
    command.env("R_DEFAULT_PACKAGES", "NULL");
    command.env("LANGUAGE", "en").env("LC_ALL", "C.UTF-8").env("TZ", "UTC");
    command.env_remove("TZDIR");
    command.current_dir(script.parent().unwrap());
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

/// The ratchet of one suite: the repository-relative ids of the scripts
/// its markers name. A marker for a script that does not exist is an error
/// (a list entry naming no script used to require nothing, silently), and
/// so is any other file in the ratchet.
pub fn load_ratchet(root: &Path, column: &Column, suite: &str) -> Result<BTreeSet<String>, String> {
    fn markers(dir: &Path, rel: &Path, suite: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e))?;
        for entry in entries {
            let path = entry.map_err(|e| format!("read {}: {}", dir.display(), e))?.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let rel = rel.join(&name);
            if path.is_dir() {
                markers(&path, &rel, suite, out)?;
            } else {
                let Some(script) = name.strip_suffix(MARKER_SUFFIX) else {
                    return Err(format!(
                        "{}: not a ratchet marker (markers are named <script>{MARKER_SUFFIX})",
                        path.display()
                    ));
                };
                out.insert(format!("{}/{}", suite, rel.with_file_name(script).display()));
            }
        }
        Ok(())
    }
    let mut ids = BTreeSet::new();
    let dir = root.join(suite).join(column.ratchet);
    if dir.is_dir() {
        markers(&dir, Path::new(""), suite, &mut ids)?;
    }
    if let Some(legacy) = column.legacy_list.filter(|l| l.starts_with(&format!("{suite}/"))) {
        let legacy = root.join(legacy);
        match std::fs::read_to_string(&legacy) {
            Ok(text) => {
                ids.extend(text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("read {}: {}", legacy.display(), e)),
        }
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

const MARKER_SUFFIX: &str = ".pass";

fn marker_path(root: &Path, column: &Column, suite: &str, id: &str) -> PathBuf {
    let rel = id.strip_prefix(&format!("{suite}/")).expect("an id inside its suite");
    root.join(suite).join(column.ratchet).join(format!("{rel}{MARKER_SUFFIX}"))
}

/// GNU R's recorded stdout, stderr and status for a case.
struct Recording {
    out: Vec<u8>,
    err: Vec<u8>,
    status: i32,
}

/// The recording at `base`, or why there is none. 4.5.0's (beside the
/// script) needs only `.Rout`: they predate the rule, and some have no
/// `.Rerr` or `.Rstatus`, which mean empty and 0. Another version's needs
/// all of `.Rout`, `.Rerr`, `.Rstatus` and `.Rsha`, the last naming the
/// script it was recorded from: a partial set is an interrupted recording
/// and a different hash a stale one, so neither is compared.
fn read_recording(base: &Path, script: &Path, version: &str) -> Result<Recording, String> {
    let read = |ext: &str| std::fs::read(format!("{}.{}", base.display(), ext)).ok();
    let status = |s: Vec<u8>| -> Result<i32, String> {
        String::from_utf8_lossy(&s).trim().parse().map_err(|_| format!("{}.Rstatus is not a status", base.display()))
    };
    if version == GATED_R_VERSION {
        let out = read("Rout").ok_or("not recorded")?;
        let status = read("Rstatus").map(status).transpose()?.unwrap_or(0);
        return Ok(Recording { out, err: read("Rerr").unwrap_or_default(), status });
    }
    let parts = ["Rout", "Rerr", "Rstatus", "Rsha"].map(read);
    let missing: Vec<&str> =
        ["Rout", "Rerr", "Rstatus", "Rsha"].iter().zip(&parts).filter(|(_, p)| p.is_none()).map(|(e, _)| *e).collect();
    if missing.len() == 4 {
        return Err("not recorded".into());
    }
    if !missing.is_empty() {
        return Err(format!("partly recorded (no .{})", missing.join(", .")));
    }
    let [Some(out), Some(err), Some(st), Some(sha)] = parts else { unreachable!() };
    let script_bytes = std::fs::read(script).map_err(|e| format!("read {}: {e}", script.display()))?;
    if String::from_utf8_lossy(&sha).trim() != sha256_hex(&script_bytes) {
        return Err("recorded from an older version of the script".into());
    }
    Ok(Recording { out, err, status: status(st)? })
}

/// One case's result.
#[derive(Debug)]
pub struct Outcome {
    /// The repository-relative path of the script.
    pub id: String,
    /// Why the selected version has no usable recording, if it has none.
    pub unrecorded: Option<String>,
    pub stdout_ok: bool,
    pub stderr_ok: bool,
    pub status_ok: bool,
    pub diff: String,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.stdout_ok && self.stderr_ok && self.status_ok
    }
}

fn run_case(bin: &Path, script: &Path) -> (Vec<u8>, Vec<u8>, i32, bool) {
    let mut child = command(bin, script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
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
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            timed_out = true;
            break -1;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    (t_out.join().unwrap(), t_err.join().unwrap(), status, timed_out)
}

fn first_diff(expected: &str, got: &str, runtime: &str) -> String {
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
                s.push_str(&format!("   {} {}: {}\n", mark, runtime, g.get(j).copied().unwrap_or("<eof>")));
            }
            return s;
        }
    }
    if expected != got {
        return "    differs in trailing newline/whitespace\n".into();
    }
    String::new()
}

fn check(root: &Path, column: &Column, suite: &str, script: &Path, o: &Options) -> Outcome {
    let id = script.strip_prefix(root).unwrap().display().to_string();
    let base = expected_base(root, suite, script, &o.version);
    let exp = match read_recording(&base, script, &o.version) {
        Ok(r) => r,
        Err(why) => {
            let diff = format!("    {why} for GNU R {}\n", o.version);
            return Outcome { id, unrecorded: Some(why), stdout_ok: false, stderr_ok: false, status_ok: false, diff };
        }
    };
    let (out, err, status, timed_out) = run_case(&o.bin, script);
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let mut diff = String::new();
    if timed_out {
        diff.push_str("    timed out\n");
    }
    // Bytes, not decoded text: invalid UTF-8 must not compare equal.
    let stdout_ok = out == exp.out && !timed_out;
    if !stdout_ok {
        diff.push_str(&first_diff(&text(&exp.out), &text(&out), column.runtime));
    }
    let stderr_ok = err == exp.err;
    let status_ok = status == exp.status;
    if stdout_ok && !stderr_ok {
        diff.push_str("    stderr differs:\n");
        diff.push_str(&first_diff(&text(&exp.err), &text(&err), column.runtime));
    }
    if !stdout_ok && !err.is_empty() && exp.err != err {
        let err = text(&err);
        let tail: Vec<&str> = err.lines().take(3).collect();
        diff.push_str(&format!("    {} stderr: {}\n", column.runtime, tail.join(" | ")));
    }
    if !status_ok {
        diff.push_str(&format!("    exit status {} (GNU R {})\n", status, exp.status));
    }
    Outcome { id, unrecorded: None, stdout_ok, stderr_ok, status_ok, diff }
}

/// What a run found.
#[derive(Debug, Default)]
pub struct Report {
    /// Every case that ran, by id.
    pub outcomes: Vec<Outcome>,
    /// The ids that passed.
    pub pass: BTreeSet<String>,
    /// Passing ids without a marker (gated runs).
    pub new_passes: Vec<String>,
    /// The markers this run added (`update`).
    pub added: Vec<PathBuf>,
}

/// Run `column`'s cases under `o`. `Err` is a failed run: a ratcheted case
/// that fails, a gated case without a usable recording, a filter that
/// names no case, a run against another version that compared nothing, or
/// a broken ratchet. Everything else is reported on stderr.
pub fn run(root: &Path, column: &Column, o: &Options) -> Result<Report, String> {
    let gated = o.version == column.gated;
    if !gated && o.update {
        return Err(format!(
            "ROBIGO_CONF_UPDATE: {}'s ratchet records runs against GNU R {}, not {}",
            column.runtime, column.gated, o.version
        ));
    }
    let filtered = |p: &Path| o.filter.as_ref().is_none_or(|f| p.display().to_string().contains(f.as_str()));

    // (suite, script) to run. The ratchet selects a `Marked` suite's cases
    // in every run, but only a gated run checks or extends it.
    let mut jobs = Vec::new();
    let mut listed: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut any_match = false;
    for suite in column.suites {
        let marks = load_ratchet(root, column, suite.dir)?;
        let run_all = suite.cases == Cases::All || o.sweep;
        let before = jobs.len();
        for p in scripts(&root.join(suite.dir), &o.features) {
            if !filtered(&p) {
                continue;
            }
            any_match = true;
            let id = p.strip_prefix(root).unwrap().display().to_string();
            if run_all || marks.contains(&id) {
                jobs.push((suite.dir, p));
            }
        }
        // A version without recordings: the reason says more than "not
        // recorded" for every case would.
        let dir = root.join(suite.dir).join(format!("R-{}", o.version));
        if jobs.len() > before && o.version != GATED_R_VERSION && !dir.is_dir() {
            return Err(format!(
                "ROBIGO_R_VERSION={}: no recordings in {}/R-{} (docs/verification.md, \"Oracles\")",
                o.version, suite.dir, o.version
            ));
        }
        if gated {
            listed.insert(suite.dir, marks);
        }
    }
    if !any_match {
        return Err(format!("no enabled conformance scripts in {} match {:?}", suite_dirs(column), o.filter));
    }

    let queue = std::sync::Mutex::new(jobs);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..o.jobs.max(1) {
            s.spawn(|| {
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some((suite, p)) = next else { break };
                    let r = check(root, column, suite, &p, o);
                    results.lock().unwrap().push(r);
                }
            });
        }
    });
    let mut outcomes: Vec<Outcome> = results.into_inner().unwrap();
    outcomes.sort_by(|a, b| a.id.cmp(&b.id));

    let ran: BTreeSet<&str> = outcomes.iter().map(|r| r.id.as_str()).collect();
    let all_listed: BTreeSet<&String> = listed.values().flatten().collect();
    // Positive control: every marker of an enabled script the filter
    // selects names a script that ran. A marker the results never match
    // (ids spelled differently) would require nothing, silently.
    let expected: Vec<&&String> = all_listed
        .iter()
        .filter(|id| {
            let p = root.join(id.as_str());
            filtered(&p) && enabled(&p, &o.features)
        })
        .collect();
    let unmatched: Vec<&&&String> = expected.iter().filter(|id| !ran.contains(id.as_str())).collect();
    if !unmatched.is_empty() {
        return Err(format!(
            "positive control: {} of {} ratchet marker(s) match no script that ran (e.g. {:?})",
            unmatched.len(),
            expected.len(),
            unmatched[0]
        ));
    }
    // A ratchet that lists nothing requires nothing, which is a mistake
    // once a column has passed anything (every column has).
    if gated && o.filter.is_none() && expected.is_empty() {
        return Err("positive control: the ratchet lists no scripts, so it requires nothing".into());
    }
    // A gated case must be judged: without a recording it would pass
    // nothing and fail nothing.
    if gated {
        let bad: Vec<String> =
            outcomes.iter().filter_map(|r| r.unrecorded.as_ref().map(|why| format!("  {} ({why})", r.id))).collect();
        if !bad.is_empty() {
            return Err(format!(
                "{} case(s) have no usable recording for GNU R {}; record them with scripts/regen-expected.sh \
                 (docs/verification.md, \"Oracles\"):\n{}",
                bad.len(),
                o.version,
                bad.join("\n")
            ));
        }
    }

    let mut report = Report::default();
    let mut regressions = Vec::new();
    for r in &outcomes {
        let passed = r.passed();
        if passed {
            report.pass.insert(r.id.clone());
            if gated && !all_listed.contains(&r.id) {
                report.new_passes.push(r.id.clone());
            }
        } else if all_listed.contains(&r.id) {
            regressions.push(r);
        }
        let mark = if passed {
            "ok  "
        } else if r.unrecorded.is_some() {
            "none"
        } else if r.stdout_ok {
            "ok* "
        } else {
            "FAIL"
        };
        eprintln!("{} {}", mark, r.id);
        if o.diff && !r.diff.is_empty() {
            eprint!("{}", r.diff);
        }
    }
    let compared = outcomes.iter().filter(|r| r.unrecorded.is_none()).count();
    let unrecorded = outcomes.len() - compared;
    eprintln!(
        "\n{} conformance{}: {}/{} pass; {} stdout, {} stderr, {} exit status match{}",
        column.runtime,
        if gated {
            format!(" against GNU R {}", o.version)
        } else {
            format!(" against GNU R {} (not ratcheted)", o.version)
        },
        report.pass.len(),
        compared,
        outcomes.iter().filter(|r| r.stdout_ok).count(),
        outcomes.iter().filter(|r| r.stderr_ok).count(),
        outcomes.iter().filter(|r| r.status_ok).count(),
        if unrecorded > 0 { format!("; {unrecorded} script(s) not recorded for {}", o.version) } else { String::new() }
    );
    if outcomes.is_empty() {
        eprintln!("(no {} case selected: the filter matches only cases it has not ratcheted)", column.runtime);
    }
    if !gated && !outcomes.is_empty() && compared == 0 {
        return Err(format!(
            "none of the {} selected case(s) has a usable recording for GNU R {}: this run compared nothing",
            outcomes.len(),
            o.version
        ));
    }
    if let (Some(path), None) = (&o.results, &o.filter) {
        write_results(path, column, &outcomes, &report.pass, &o.version);
    }
    if !report.new_passes.is_empty() {
        eprintln!("newly passing (not yet ratcheted):");
        for p in &report.new_passes {
            eprintln!("  {}", p);
        }
    }
    if o.update {
        // Only add markers: no existing file is rewritten, so a filtered
        // run can ratchet the cases it ran, and two branches never touch
        // the same marker.
        for id in &report.pass {
            let suite = column.suites.iter().find(|s| id.starts_with(&format!("{}/", s.dir))).unwrap();
            let marker = marker_path(root, column, suite.dir, id);
            if marker.exists() {
                continue;
            }
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, "").unwrap();
            report.added.push(marker);
        }
        if !report.added.is_empty() {
            eprintln!(
                "\nratcheted {} new script(s) under {}/{}",
                report.added.len(),
                suite_dirs(column),
                column.ratchet
            );
        }
    }
    if !regressions.is_empty() {
        let mut msg = format!("{} ratcheted script(s) regressed:\n", regressions.len());
        for r in regressions {
            msg.push_str(&format!("  {}\n{}", r.id, r.diff));
        }
        return Err(msg);
    }
    report.outcomes = outcomes;
    Ok(report)
}

fn suite_dirs(column: &Column) -> String {
    let dirs: Vec<&str> = column.suites.iter().map(|s| s.dir).collect();
    if dirs.len() == 1 { dirs[0].to_string() } else { format!("{{{}}}", dirs.join(",")) }
}

/// The run as `{"summary": {...}, "results": [{"id", "pass"}]}`, the shape
/// of scripts/corpus.py's result files, which scripts/history.py reads.
fn write_results(path: &Path, column: &Column, results: &[Outcome], pass: &BTreeSet<String>, version: &str) {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let rows: Vec<String> =
        results.iter().map(|r| format!("{{\"id\": {}, \"pass\": {}}}", q(&r.id), pass.contains(&r.id))).collect();
    // Only another version's results name theirs, and scripts/history.py
    // skips summaries that do: the progress page reads the gated ones.
    let r_version = if version == column.gated { String::new() } else { format!(", \"r_version\": {}", q(version)) };
    let doc = format!(
        "{{\"summary\": {{\"corpus\": {}{}, \"cases\": {}, \"pass\": {}}},\n\"results\": [\n{}\n]}}\n",
        q(column.corpus),
        r_version,
        results.iter().filter(|r| r.unrecorded.is_none()).count(),
        pass.len(),
        rows.join(",\n")
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, doc).unwrap_or_else(|e| panic!("writing {}: {}", path.display(), e));
}

/// SHA-256 of `data` in lowercase hex, as `sha256sum` prints it
/// (scripts/regen-expected.sh writes `.Rsha` with it). FIPS 180-4.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
