//! The runner on fixture trees, with fake binaries in place of a runtime:
//! `cat` (a case passes when its recorded stdout is its own source), a
//! stub that fails everything the way today's `robigo-rt` does, and a
//! `cat` that takes a second.

use robigo_conformance::{
    Column, OLD, Options, RT, enabled, expected_base, load_ratchet, r_version, run, scripts, sha256_hex,
};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The fake binaries, written once before any test spawns one: a binary
/// written while another thread forks can fail to exec ("text file busy").
/// A runtime that fails every case at once, and whose boot probe (the
/// runner's record of what it does not run, ROBIGO_BOOT_VERBOSE) takes 3 s.
fn slow_probe() -> PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let p = fakes().0.parent().unwrap().join("slow-probe");
        std::fs::write(&p, "#!/bin/sh\n[ -n \"$ROBIGO_BOOT_VERBOSE\" ] && sleep 3\nexit 1\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    })
    .clone()
}

fn fakes() -> &'static (PathBuf, PathBuf, PathBuf, PathBuf) {
    static FAKES: OnceLock<(PathBuf, PathBuf, PathBuf, PathBuf)> = OnceLock::new();
    FAKES.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        // in cargo's target directory, so the fakes go with it (`cargo
        // clean`), not in /tmp: a test process has no exit hook to remove
        // them, so each run removes the ones of test processes that are gone
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR"));
        sweep_fakes(parent);
        // the pid for the sweep, and a time so that a reused pid never
        // adopts (or has swept under it) another run's directory
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = parent.join(format!("{FAKES_PREFIX}{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(parent).unwrap();
        std::fs::create_dir(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let cat = write("cat", "#!/bin/sh\nexec cat \"$1\"\n");
        let stub = write("stub", "#!/bin/sh\necho 'robigo-rt: the G3 runtime is not implemented yet' >&2\nexit 1\n");
        let slow = write("slow", "#!/bin/sh\nsleep 1\nexec cat \"$1\"\n");
        // `cat`, noting each case's path as it starts in a log beside
        // itself (`order_log`): one test uses it
        let order = write("order", "#!/bin/sh\necho \"$1\" >> \"$0.log\"\nexec cat \"$1\"\n");
        (cat, stub, slow, order)
    })
}

const FAKES_PREFIX: &str = "robigo-conformance-fakes-";

/// Removes `parent`'s fakes directories whose test process no longer runs
/// (`robigo-conformance-fakes-<pid>-<time>` with no `/proc/<pid>`; without
/// a /proc, none).
fn sweep_fakes(parent: &Path) {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(pid) =
            name.to_str().and_then(|n| n.strip_prefix(FAKES_PREFIX)).map(|r| r.split('-').next().unwrap_or(""))
        else {
            continue;
        };
        if pid.parse::<u32>().is_ok() && !Path::new("/proc").join(pid).exists() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[test]
fn a_gone_test_process_leaves_no_fakes_behind() {
    if !Path::new("/proc/self").exists() {
        return; // no process table: sweep_fakes removes nothing there
    }
    let parent = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    // pid_max is at most 2^22, so this pid never runs
    let gone = parent.join(format!("{FAKES_PREFIX}4999999-1"));
    let live = parent.join(format!("{FAKES_PREFIX}{}-1", std::process::id()));
    let other = parent.join("something-else-1");
    for d in [&gone, &live, &other] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("cat"), "x").unwrap();
    }
    sweep_fakes(&parent);
    assert!(!gone.exists());
    assert!(live.exists());
    assert!(other.exists());
    std::fs::remove_dir_all(&parent).unwrap();
}

fn cat() -> PathBuf {
    fakes().0.clone()
}

fn stub() -> PathBuf {
    fakes().1.clone()
}

fn slow() -> PathBuf {
    fakes().2.clone()
}

fn order() -> PathBuf {
    fakes().3.clone()
}

/// Where `order` notes the cases it starts.
fn order_log() -> PathBuf {
    PathBuf::from(format!("{}.log", order().display()))
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        fakes();
        let dir = std::env::temp_dir().join(format!("robigo-conformance-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Fixture(dir)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    /// A case `id` (repository relative) with source `src`.
    fn case(&self, id: &str, src: &str) -> &Self {
        let p = self.0.join(id);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
        self
    }

    /// GNU R `version`'s recording of case `id` in `suite`, as
    /// scripts/regen-expected.sh writes it; `parts` limits the files.
    fn record_parts(&self, suite: &str, id: &str, version: &str, out: &str, parts: &[&str]) -> &Self {
        let script = self.0.join(id);
        let base = expected_base(&self.0, suite, &script, version);
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        let sha = sha256_hex(&std::fs::read(&script).unwrap());
        for (ext, body) in
            [("Rout", out.to_string()), ("Rerr", String::new()), ("Rstatus", "0\n".into()), ("Rsha", sha)]
        {
            if parts.contains(&ext) {
                std::fs::write(format!("{}.{ext}", base.display()), body).unwrap();
            }
        }
        self
    }

    fn record(&self, suite: &str, id: &str, version: &str, out: &str) -> &Self {
        let parts: &[&str] =
            if version == "4.5.0" { &["Rout", "Rerr", "Rstatus"] } else { &["Rout", "Rerr", "Rstatus", "Rsha"] };
        self.record_parts(suite, id, version, out, parts)
    }

    fn mark(&self, column: &Column, suite: &str, id: &str) -> &Self {
        let rel = id.strip_prefix(&format!("{suite}/")).unwrap();
        let m = self.0.join(suite).join(column.ratchet).join(format!("{rel}.pass"));
        std::fs::create_dir_all(m.parent().unwrap()).unwrap();
        std::fs::write(m, "").unwrap();
        self
    }

    fn marker(&self, column: &Column, suite: &str, rel: &str) -> PathBuf {
        self.0.join(suite).join(column.ratchet).join(format!("{rel}.pass"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn opts(column: &Column, bin: PathBuf) -> Options {
    Options {
        bin,
        version: column.gated.to_string(),
        filter: None,
        diff: false,
        update: false,
        sweep: false,
        features: Vec::new(),
        results: None,
        jobs: 2,
        timeout: std::time::Duration::from_secs(20),
        case_timeouts: Default::default(),
        times: None,
        slots: None,
        env: Vec::new(),
    }
}

/// A tree with one `tests/rt` case and two `tests/r` cases, all recorded
/// for 4.6.2 so that `cat` passes them.
fn rt_tree(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.case("tests/rt/smoke/one.R", "1\n").record("tests/rt", "tests/rt/smoke/one.R", "4.6.2", "1\n");
    f.case("tests/r/basics/a.R", "a\n").record("tests/r", "tests/r/basics/a.R", "4.6.2", "a\n");
    f.case("tests/r/basics/b.R", "b\n").record("tests/r", "tests/r/basics/b.R", "4.6.2", "b\n");
    f
}

fn ids(r: &robigo_conformance::Report) -> Vec<&str> {
    r.outcomes.iter().map(|o| o.id.as_str()).collect()
}

/// A runtime that runs nothing fails every case and ratchets nothing; with
/// no marker either, the run fails: an empty ratchet requires nothing.
#[test]
fn a_runtime_that_runs_nothing_fails_every_case_and_ratchets_nothing() {
    let f = rt_tree("stub");
    let mut o = opts(&RT, stub());
    o.update = true;
    let err = run(f.root(), &RT, &o).expect_err("an unfiltered rt run with an empty ratchet fails");
    assert!(err.contains("requires nothing"), "{err}");
    assert!(!f.root().join("tests/rt/passing-rt").exists());
    // filtered, the run reports the case and does not judge the ratchet
    o.filter = Some("tests/rt/smoke".into());
    let r = run(f.root(), &RT, &o).unwrap();
    assert_eq!(ids(&r), ["tests/rt/smoke/one.R"], "tests/r runs only its marked cases");
    assert!(r.pass.is_empty() && r.added.is_empty() && r.new_passes.is_empty());
    assert!(!r.outcomes[0].passed() && r.outcomes[0].unrecorded.is_none());
}

#[test]
fn a_passing_case_is_ratcheted_only_on_request_and_then_held() {
    let f = rt_tree("ratchet");
    // filtered: the ratchet is empty until the update
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/rt/smoke".into());
    let r = run(f.root(), &RT, &o).unwrap();
    assert_eq!(r.new_passes, ["tests/rt/smoke/one.R"]);
    assert!(r.added.is_empty() && !f.marker(&RT, "tests/rt", "smoke/one.R").exists());

    o.update = true;
    let r = run(f.root(), &RT, &o).unwrap();
    let marker = f.marker(&RT, "tests/rt", "smoke/one.R");
    assert_eq!(r.added, std::slice::from_ref(&marker));
    assert_eq!(std::fs::read(&marker).unwrap(), b"", "a marker is an empty file");
    // The old column's ratchet is untouched.
    assert!(!f.root().join("tests/rt/passing").exists() && !f.root().join("tests/r/passing").exists());

    let err = run(f.root(), &RT, &opts(&RT, stub())).expect_err("a ratcheted case that fails fails the run");
    assert!(err.contains("1 ratcheted script(s) regressed") && err.contains("tests/rt/smoke/one.R"), "{err}");
    assert!(marker.exists(), "a failing run never removes a marker");
}

/// rt's `tests/r` column: a marked case runs and is held; the sweep runs the
/// others and ratchets them into `tests/r/passing-rt/`.
#[test]
fn tests_r_runs_its_marked_cases_and_the_sweep_runs_the_rest() {
    let f = rt_tree("sweep");
    f.mark(&RT, "tests/r", "tests/r/basics/a.R");
    let r = run(f.root(), &RT, &opts(&RT, cat())).unwrap();
    assert_eq!(ids(&r), ["tests/r/basics/a.R", "tests/rt/smoke/one.R"]);
    let err = run(f.root(), &RT, &opts(&RT, stub())).unwrap_err();
    assert!(err.contains("1 ratcheted script(s) regressed"), "{err}");
    assert!(err.contains("tests/r/basics/a.R"), "{err}");

    let mut o = opts(&RT, cat());
    o.sweep = true;
    o.update = true;
    let r = run(f.root(), &RT, &o).unwrap();
    assert_eq!(ids(&r), ["tests/r/basics/a.R", "tests/r/basics/b.R", "tests/rt/smoke/one.R"]);
    assert!(f.marker(&RT, "tests/r", "basics/b.R").exists());
    assert!(f.marker(&RT, "tests/rt", "smoke/one.R").exists());
}

#[test]
fn a_gated_case_needs_a_complete_current_recording() {
    for (what, parts, edit, needle) in [
        ("missing", &[][..], false, "not recorded"),
        ("partial", &["Rout", "Rerr", "Rstatus"][..], false, "partly recorded (no .Rsha)"),
        ("partial2", &["Rout", "Rsha"][..], false, "partly recorded (no .Rerr, .Rstatus)"),
        ("stale", &["Rout", "Rerr", "Rstatus", "Rsha"][..], true, "older version of the script"),
    ] {
        let f = Fixture::new(&format!("gated-{what}"));
        f.case("tests/rt/x.R", "1\n").record_parts("tests/rt", "tests/rt/x.R", "4.6.2", "1\n", parts);
        std::fs::create_dir_all(f.root().join("tests/rt/R-4.6.2")).unwrap();
        // (marked: an unfiltered run with an empty ratchet fails first)
        f.mark(&RT, "tests/rt", "tests/rt/x.R");
        if edit {
            f.case("tests/rt/x.R", "2\n");
        }
        let err = run(f.root(), &RT, &opts(&RT, cat())).expect_err(what);
        assert!(err.contains("no usable recording for GNU R 4.6.2") && err.contains(needle), "{what}: {err}");
    }
}

/// A run against a version the column is not gated on reports: an
/// incomplete recording is "not recorded", and a run that compared nothing
/// fails, as does one asked to ratchet.
#[test]
fn another_version_only_reports_and_must_compare_something() {
    let f = Fixture::new("other-version");
    f.case("tests/r/a.R", "a\n").record("tests/r", "tests/r/a.R", "4.5.0", "a\n");
    f.case("tests/r/b.R", "b\n").record("tests/r", "tests/r/b.R", "4.5.0", "b\n");
    f.mark(&OLD, "tests/r", "tests/r/a.R").mark(&OLD, "tests/r", "tests/r/b.R");
    f.record("tests/r", "tests/r/a.R", "4.6.2", "a\n");
    f.record_parts("tests/r", "tests/r/b.R", "4.6.2", "b\n", &["Rout"]);
    let mut o = opts(&OLD, stub());
    o.version = "4.6.2".into();
    let r = run(f.root(), &OLD, &o).expect("no ratchet is checked against another version");
    let b = r.outcomes.iter().find(|x| x.id == "tests/r/b.R").unwrap();
    assert_eq!(b.unrecorded.as_deref(), Some("partly recorded (no .Rerr, .Rstatus, .Rsha)"));

    std::fs::remove_file(f.root().join("tests/r/R-4.6.2/a.Rsha")).unwrap();
    let err = run(f.root(), &OLD, &o).expect_err("a run that compared nothing fails");
    assert!(err.contains("compared nothing"), "{err}");

    o.update = true;
    let err = run(f.root(), &OLD, &o).unwrap_err();
    assert!(err.contains("ROBIGO_CONF_UPDATE"), "{err}");

    o.update = false;
    o.version = "4.7.0".into();
    let err = run(f.root(), &OLD, &o).unwrap_err();
    assert!(err.contains("no recordings in tests/r/R-4.7.0"), "{err}");

    // robigo-rt against 4.5.0: tests/rt has no 4.5.0 recordings at all.
    let f = rt_tree("rt-old-version");
    let mut o = opts(&RT, cat());
    o.version = "4.5.0".into();
    let err = run(f.root(), &RT, &o).unwrap_err();
    assert!(err.contains("compared nothing"), "{err}");
}

/// The old column's recordings beside each script predate the complete-set
/// rule: `.Rout` alone is a recording, a missing `.Rstatus` means 0.
#[test]
fn the_old_columns_recordings_need_only_rout() {
    let f = Fixture::new("old-legacy");
    f.case("tests/r/a.R", "a\n").record_parts("tests/r", "tests/r/a.R", "4.5.0", "a\n", &["Rout"]);
    f.mark(&OLD, "tests/r", "tests/r/a.R");
    let r = run(f.root(), &OLD, &opts(&OLD, cat())).unwrap();
    assert!(r.pass.contains("tests/r/a.R"));
    std::fs::remove_file(f.root().join("tests/r/a.Rout")).unwrap();
    let err = run(f.root(), &OLD, &opts(&OLD, cat())).unwrap_err();
    assert!(err.contains("not recorded"), "{err}");
}

#[test]
fn an_empty_ratchet_is_an_error_in_every_column() {
    let f = Fixture::new("empty-ratchet");
    f.case("tests/r/a.R", "a\n").record("tests/r", "tests/r/a.R", "4.5.0", "a\n");
    let err = run(f.root(), &OLD, &opts(&OLD, cat())).unwrap_err();
    assert!(err.contains("requires nothing"), "{err}");
    let f = rt_tree("empty-ratchet-rt");
    let err = run(f.root(), &RT, &opts(&RT, cat())).unwrap_err();
    assert!(err.contains("requires nothing"), "{err}");
}

/// The cases `ROBIGO_CONF_TIMEOUTS` names are the slow ones: they start
/// first, so a run with several workers does not wait on one that began
/// late. One worker here, so the start order is the order they run in.
#[test]
fn the_cases_with_a_longer_timeout_start_first() {
    let f = rt_tree("slow-first");
    f.case("tests/rt/smoke/two.R", "2\n").record("tests/rt", "tests/rt/smoke/two.R", "4.6.2", "2\n");
    let log = order_log();
    let mut o = opts(&RT, order());
    o.filter = Some("tests/rt/smoke".into());
    o.jobs = 1;
    let first = |o: &Options| {
        let _ = std::fs::remove_file(&log);
        run(f.root(), &RT, o).unwrap();
        let started = std::fs::read_to_string(&log).unwrap();
        assert_eq!(started.lines().count(), 2, "{started}");
        started.lines().next().unwrap().rsplit('/').next().unwrap().to_string()
    };
    assert_eq!(first(&o), "two.R", "the workers take the last case first");
    o.case_timeouts.insert("tests/rt/smoke/one.R".into(), std::time::Duration::from_secs(30));
    assert_eq!(first(&o), "one.R");
}

/// `ROBIGO_CONF_TIMES_REPORT`: one line per case run, slowest first, with
/// the limit it ran under and whether it passed, also when the run fails.
#[test]
fn the_times_report_lists_every_case_run_with_its_limit() {
    let f = rt_tree("times");
    f.case("tests/rt/smoke/two.R", "2\n").record("tests/rt", "tests/rt/smoke/two.R", "4.6.2", "2\n");
    let report = f.root().join("times.tsv");
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/rt/smoke".into());
    o.case_timeouts.insert("tests/rt/smoke/one.R".into(), std::time::Duration::from_secs(30));
    o.times = Some(report.clone());
    let _ = run(f.root(), &RT, &o);
    let text = std::fs::read_to_string(&report).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("# secs\tlimit\tstatus\tcase"));
    let rows: Vec<Vec<&str>> = lines.map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 2, "{text}");
    let secs: Vec<f64> = rows.iter().map(|r| r[0].parse().unwrap()).collect();
    assert!(secs[0] >= secs[1], "slowest first: {text}");
    let by_case: std::collections::BTreeMap<&str, (&str, &str)> = rows.iter().map(|r| (r[3], (r[1], r[2]))).collect();
    assert_eq!(by_case["tests/rt/smoke/one.R"].0, "30");
    assert_eq!(by_case["tests/rt/smoke/two.R"].0, "20");
    assert!(by_case.values().all(|(_, s)| *s == "pass" || *s == "fail"));
    // a runtime that fails every case still writes it
    let _ = std::fs::remove_file(&report);
    let mut o = opts(&RT, stub());
    o.filter = Some("tests/rt/smoke".into());
    o.times = Some(report.clone());
    let _ = run(f.root(), &RT, &o);
    let text = std::fs::read_to_string(&report).unwrap();
    assert_eq!(text.lines().filter(|l| l.ends_with(".R")).count(), 2, "{text}");
    assert!(text.lines().skip(1).all(|l| l.split('\t').nth(2) == Some("fail")), "{text}");
    // a report whose directory does not exist yet: it is made
    let nested = f.root().join("not").join("yet").join("times.tsv");
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/rt/smoke".into());
    o.times = Some(nested.clone());
    let _ = run(f.root(), &RT, &o);
    assert!(nested.is_file(), "no report in a new directory");
}

/// A case's time is its own run: the boot probe the first failure starts
/// (which the other failing cases wait for) is not counted in any case's.
#[test]
fn a_case_time_leaves_out_the_boot_probe() {
    let f = rt_tree("times-probe");
    f.case("tests/rt/smoke/two.R", "2\n").record("tests/rt", "tests/rt/smoke/two.R", "4.6.2", "2\n");
    let report = f.root().join("times.tsv");
    let mut o = opts(&RT, slow_probe());
    o.filter = Some("tests/rt/smoke".into());
    o.times = Some(report.clone());
    let _ = run(f.root(), &RT, &o);
    let text = std::fs::read_to_string(&report).unwrap();
    let secs: Vec<f64> = text.lines().skip(1).map(|l| l.split('\t').next().unwrap().parse().unwrap()).collect();
    assert_eq!(secs.len(), 2, "{text}");
    assert!(secs.iter().all(|s| *s < 2.0), "the 3 s probe counted in a case's time: {text}");
}

/// A case's slot goes with its process, not with its output pipes: a
/// descendant that keeps stdout open (a background `sleep`) must not hold
/// the slot, or one slot would serialize on it (or deadlock on a nested
/// run). Two cases, one slot, each leaving a 3 s `sleep` on its stdout:
/// the second starts before the first's sleep ends (the cases' own stamps,
/// no wall-clock margin).
#[test]
fn a_cases_slot_is_released_when_its_process_ends() {
    use std::os::unix::fs::PermissionsExt;
    let pool = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slots-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pool);
    let f = Fixture::new("slot-pipe");
    for c in ["one", "two"] {
        f.case(&format!("tests/rt/smoke/{c}.R"), "x\n").record(
            "tests/rt",
            &format!("tests/rt/smoke/{c}.R"),
            "4.6.2",
            "1\n",
        );
    }
    let log = f.root().join("stamps");
    let fake = f.root().join("lingering-fake");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$1 start $(date +%s.%N)\" >> {}\necho 1\nsleep 3 &\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut o = opts(&RT, fake);
    o.filter = Some("tests/rt/smoke".into());
    o.jobs = 2;
    o.slots = Some((pool.clone(), 1));
    let r = run(f.root(), &RT, &o).unwrap();
    assert_eq!(r.outcomes.len(), 2);
    let mut starts: Vec<f64> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| l.split_whitespace().nth(2).unwrap().parse().unwrap())
        .collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(starts.len(), 2);
    assert!(starts[1] - starts[0] < 3.0, "the slot waited for the first case's pipes: {starts:?}");
    let _ = std::fs::remove_dir_all(&pool);
}

/// A case starts with SIGINT at its default whatever the runner inherited:
/// a runner under an ignored SIGINT (a background shell's `cmd &`) once
/// passed SIG_IGN on, and a case that raises one failed (interrupt.R).
#[test]
#[allow(unsafe_code)]
fn a_case_starts_with_sigint_at_its_default() {
    use std::os::unix::fs::PermissionsExt;
    if !Path::new("/proc/self/status").exists() {
        return; // the fake reads SigIgn there
    }
    let f = Fixture::new("sigint");
    f.case("tests/rt/smoke/sig.R", "x\n").record("tests/rt", "tests/rt/smoke/sig.R", "4.6.2", "0\n");
    // a runtime that prints whether SIGINT is ignored in it (SigIgn's bit 2)
    let fake = f.root().join("sigint-fake");
    std::fs::write(&fake, "#!/bin/sh\nv=$(sed -n 's/^SigIgn:[[:space:]]*//p' /proc/$$/status)\necho $(( 0x$v & 2 ))\n")
        .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut o = opts(&RT, fake);
    o.filter = Some("tests/rt/smoke/sig.R".into());
    // the process's SIGINT ignored for the run, restored on drop (a panic
    // included: the binary's later tests must not inherit it)
    struct Restore(libc::sighandler_t);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: signal() only changes this process's disposition.
            unsafe { libc::signal(libc::SIGINT, self.0) };
        }
    }
    // SAFETY: as above; the other tests' cases go through the same reset.
    let _restore = Restore(unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) });
    let r = run(f.root(), &RT, &o).unwrap();
    assert!(r.outcomes[0].passed(), "the case saw SIGINT ignored: {}", r.outcomes[0].diff);
}

/// A fake runtime that notes each case's own start and end (`date +%s.%N`)
/// in `log`, sleeping `secs` in between, then prints the case.
fn stamping_fake(root: &Path, log: &Path, secs: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let fake = root.join(format!("stamp-fake-{secs}"));
    let body = format!(
        "#!/bin/sh\necho \"$1 start $(date +%s.%N)\" >> {log}\nsleep {secs}\necho \"$1 end $(date +%s.%N)\" >> {log}\nexec cat \"$1\"\n",
        log = log.display()
    );
    std::fs::write(&fake, body).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

/// Each case's (start, end) from a stamping fake's log.
fn intervals(log: &Path) -> Vec<(f64, f64)> {
    let text = std::fs::read_to_string(log).unwrap();
    let mut open = std::collections::HashMap::new();
    let mut out = Vec::new();
    for l in text.lines() {
        let w: Vec<&str> = l.split_whitespace().collect();
        let at: f64 = w[2].parse().unwrap();
        if w[1] == "start" {
            open.insert(w[0].to_string(), at);
        } else {
            out.push((open[w[0]], at));
        }
    }
    out
}

/// The machine-wide slot pool (`ROBIGO_CONF_SLOTS_DIR`), asserted without a
/// wall-clock margin: two runs at once, two cases each, one slot. No two
/// cases' own intervals (stamped by the cases) overlap; with as many slots
/// as cases they do. Limits are far out of load's reach (60 s).
#[test]
fn runs_sharing_a_slot_pool_take_turns() {
    for (slots, overlap) in [(1, false), (4, true)] {
        let pool = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slots-{}-{slots}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pool);
        let log = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stamps-{}-{slots}", std::process::id()));
        let _ = std::fs::remove_file(&log);
        let trees: Vec<Fixture> = (0..2)
            .map(|k| {
                let f = rt_tree(&format!("slots-{slots}-{k}"));
                f.case("tests/rt/smoke/two.R", "2\n").record("tests/rt", "tests/rt/smoke/two.R", "4.6.2", "2\n");
                f
            })
            .collect();
        std::thread::scope(|s| {
            for f in &trees {
                let (pool, log) = (pool.clone(), log.clone());
                s.spawn(move || {
                    let mut o = opts(&RT, stamping_fake(f.root(), &log, "1"));
                    o.filter = Some("tests/rt/smoke".into());
                    o.jobs = 2;
                    o.timeout = std::time::Duration::from_secs(60);
                    o.slots = Some((pool, slots));
                    let r = run(f.root(), &RT, &o).unwrap();
                    assert!(
                        r.outcomes.iter().all(|c| c.passed()),
                        "{:?}",
                        r.outcomes.iter().map(|c| &c.diff).collect::<Vec<_>>()
                    );
                });
            }
        });
        let mut iv = intervals(&log);
        assert_eq!(iv.len(), 4);
        iv.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let overlaps = iv.windows(2).any(|w| w[1].0 < w[0].1);
        assert_eq!(overlaps, overlap, "{slots} slot(s): {iv:?}");
        let _ = std::fs::remove_dir_all(&pool);
        let _ = std::fs::remove_file(&log);
    }
}

/// An existing pool directory that others can write in is refused (a slot
/// file there could be replaced), not used: the run stops ("not a private
/// directory", from a worker: the scope re-raises it as its own panic; the
/// other pool tests show a private one works).
#[test]
#[should_panic]
fn a_pool_directory_others_can_write_in_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let pool = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slots-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pool);
    std::fs::create_dir_all(&pool).unwrap();
    std::fs::set_permissions(&pool, std::fs::Permissions::from_mode(0o777)).unwrap();
    let f = rt_tree("slots-open");
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/rt/smoke".into());
    o.slots = Some((pool, 1));
    let _ = run(f.root(), &RT, &o);
}

/// A case's clock starts at its slot, not while it waits for one: a run
/// holds the only slot with a 3 s case; once that case has stamped its
/// start, a second run's quick case, at a 2 s limit, waits for the slot and
/// passes, and its reported time leaves the wait out. (The order is made
/// by the test, not by a race; the slow case's own limit, 60 s, is far out
/// of load's reach.)
#[test]
fn a_cases_clock_starts_when_it_takes_its_slot() {
    use std::os::unix::fs::PermissionsExt;
    let pool = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slots-clock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pool);
    let log = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stamps-clock-{}", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let slow = rt_tree("slots-clock-slow");
    let quick = rt_tree("slots-clock-quick");
    let report = quick.root().join("times.tsv");
    let fake = |f: &Fixture, secs: &str| {
        let p = f.root().join("clock-fake");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"$1 start $(date +%s.%N)\" >> {log}\nsleep {secs}\nexec cat \"$1\"\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let mut so = opts(&RT, fake(&slow, "3"));
    so.filter = Some("tests/rt/smoke".into());
    so.timeout = std::time::Duration::from_secs(60);
    so.slots = Some((pool.clone(), 1));
    let mut qo = opts(&RT, fake(&quick, "0"));
    qo.filter = Some("tests/rt/smoke".into());
    qo.timeout = std::time::Duration::from_secs(2);
    qo.slots = Some((pool.clone(), 1));
    qo.times = Some(report.clone());
    let (asked, quick_result) = std::thread::scope(|s| {
        let h = s.spawn(|| run(slow.root(), &RT, &so).unwrap());
        // the slow case holds the slot once it has stamped its start (a
        // bound, so that a slow run that never starts fails here, saying
        // what it saw, instead of hanging the test binary)
        let since = std::time::Instant::now();
        while !std::fs::read_to_string(&log).is_ok_and(|t| t.contains(" start ")) {
            assert!(
                since.elapsed() < std::time::Duration::from_secs(60),
                "the slow case did not stamp its start in 60 s; its log: {:?}; the slow run finished: {}",
                std::fs::read_to_string(&log).ok(),
                h.is_finished()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let asked = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let q = run(quick.root(), &RT, &qo).unwrap();
        assert!(h.join().unwrap().outcomes.iter().all(|c| c.passed()));
        (asked, q)
    });
    assert!(
        quick_result.outcomes.iter().all(|c| c.passed()),
        "the wait counted: {:?}",
        quick_result.outcomes.iter().map(|c| &c.diff).collect::<Vec<_>>()
    );
    let starts: Vec<f64> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| l.split_whitespace().nth(2).unwrap().parse().unwrap())
        .collect();
    let quick_started = starts.iter().cloned().fold(f64::MIN, f64::max);
    let waited = quick_started - asked;
    let times = std::fs::read_to_string(&report).unwrap();
    let reported: f64 = times.lines().nth(1).unwrap().split('\t').next().unwrap().parse().unwrap();
    // it waited for the slot, longer than its 2 s limit, and its time does
    // not hold that wait
    assert!(waited > 2.0, "the quick case did not wait for the slot: {waited}");
    assert!(reported < waited, "the quick case's time ({reported} s) holds its wait ({waited} s)");
    let _ = std::fs::remove_dir_all(&pool);
    let _ = std::fs::remove_file(&log);
}

/// `ROBIGO_CONF_TIMEOUTS` gives the case it names its own timeout. It names
/// the case by its id, as `check` looks it up: any other name, even a path
/// to the same file, is refused, since it would never apply.
#[test]
fn a_case_timeout_applies_to_the_case_its_id_names() {
    let f = rt_tree("case-timeouts");
    let mut o = opts(&RT, slow());
    o.filter = Some("tests/rt/smoke".into());
    o.timeout = std::time::Duration::from_millis(300);
    let r = run(f.root(), &RT, &o).unwrap();
    assert!(!r.outcomes[0].passed() && r.outcomes[0].diff.contains("timed out"), "{}", r.outcomes[0].diff);

    o.case_timeouts.insert("tests/rt/smoke/one.R".into(), std::time::Duration::from_secs(30));
    // a case the filter leaves out may be named too: the step's table stays valid in a filtered run
    o.case_timeouts.insert("tests/r/basics/b.R".into(), std::time::Duration::from_secs(30));
    let r = run(f.root(), &RT, &o).unwrap();
    assert_eq!(ids(&r), ["tests/rt/smoke/one.R"]);
    assert!(r.outcomes[0].passed(), "{}", r.outcomes[0].diff);

    for name in ["./tests/rt/smoke/one.R", "tests//rt/smoke/one.R", "tests/rt/smoke/one.Rout", "tests/rt/smoke/gone.R"]
    {
        o.case_timeouts.clear();
        o.case_timeouts.insert(name.into(), std::time::Duration::from_secs(30));
        let err = run(f.root(), &RT, &o).expect_err(name);
        assert!(err.contains(&format!("ROBIGO_CONF_TIMEOUTS names {name}, which is no case's id")), "{err}");
    }
}

/// A filter must name a case of the column's suites; one that names only
/// cases the column runs when marked selects nothing, which is fine (a
/// `cargo test --workspace` with a `tests/r` filter runs both columns).
#[test]
fn a_filter_names_cases_that_exist() {
    let f = rt_tree("filter");
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/r/basics/b".into());
    let r = run(f.root(), &RT, &o).unwrap();
    assert!(r.outcomes.is_empty());
    o.filter = Some("tests/r/nonesuch".into());
    let err = run(f.root(), &RT, &o).unwrap_err();
    assert!(err.contains("no enabled conformance scripts"), "{err}");
}

#[test]
fn the_results_file_names_the_column() {
    let f = rt_tree("results");
    f.mark(&RT, "tests/rt", "tests/rt/smoke/one.R");
    let mut o = opts(&RT, cat());
    let path = f.root().join("out/results.json");
    o.results = Some(path.clone());
    run(f.root(), &RT, &o).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("{\"summary\": {\"corpus\": \"conformance-rt\", \"cases\": 1, \"pass\": 1}"), "{text}");
}

#[test]
fn r_version_is_the_gated_one_only_when_unset() {
    assert_eq!(r_version(None, &OLD).unwrap(), "4.5.0");
    assert_eq!(r_version(None, &RT).unwrap(), "4.6.2");
    assert_eq!(r_version(Some(OsStr::new("4.6.2")), &OLD).unwrap(), "4.6.2");
    for bad in ["", "4.6.", "x", "4.6.2 "] {
        assert!(r_version(Some(OsStr::new(bad)), &OLD).is_err(), "{bad:?}");
    }
    use std::os::unix::ffi::OsStrExt;
    let err = r_version(Some(OsStr::from_bytes(b"4.6.\xff")), &OLD).unwrap_err();
    assert!(err.contains("not UTF-8"), "{err}");
}

/// 4.5.0's recordings sit beside each script; another version's mirror the
/// suite under `R-<version>/`, so adding a version touches no 4.5.0 file and
/// a case keeps its identity (its path) across versions.
#[test]
fn expected_sets_by_version() {
    let root = Path::new("/repo");
    let script = root.join("tests/r/strings/x.R");
    assert_eq!(expected_base(root, "tests/r", &script, "4.5.0"), root.join("tests/r/strings/x"));
    assert_eq!(expected_base(root, "tests/r", &script, "4.6.2"), root.join("tests/r/R-4.6.2/strings/x"));
    let script = root.join("tests/rt/smoke/one.R");
    assert_eq!(expected_base(root, "tests/rt", &script, "4.6.2"), root.join("tests/rt/R-4.6.2/smoke/one"));
}

#[test]
fn ratchet_reads_markers() {
    let f = Fixture::new("read");
    f.case("tests/r/basics/arith.R", "").mark(&OLD, "tests/r", "tests/r/basics/arith.R");
    let listed = load_ratchet(f.root(), &OLD, "tests/r").unwrap();
    assert_eq!(listed, ["tests/r/basics/arith.R".to_string()].into_iter().collect());
    assert!(load_ratchet(f.root(), &RT, "tests/r").unwrap().is_empty(), "the columns have separate ratchets");
}

/// The failure mode of the old single list: an entry naming no script did
/// nothing, silently. A marker must fail loudly instead.
#[test]
fn ratchet_rejects_marker_for_missing_script() {
    let f = Fixture::new("dangling");
    f.case("tests/r/basics/arith.R", "").mark(&RT, "tests/r", "tests/r/basics/ghost.R");
    let err = load_ratchet(f.root(), &RT, "tests/r").expect_err("a dangling marker must be an error");
    assert!(err.contains("does not exist") && err.contains("basics/ghost.R"), "{}", err);
}

#[test]
fn ratchet_rejects_stray_file() {
    let f = Fixture::new("stray");
    std::fs::create_dir_all(f.root().join("tests/r/passing")).unwrap();
    std::fs::write(f.root().join("tests/r/passing/notes.txt"), "not a marker").unwrap();
    let err = load_ratchet(f.root(), &OLD, "tests/r").expect_err("a stray file in the ratchet must be an error");
    assert!(err.contains("not a ratchet marker"), "{}", err);
}

/// Branches based on the old layout still work: the single list is read too
/// until `scripts/ratchet.py migrate` converts it.
#[test]
fn ratchet_reads_legacy_list() {
    let f = Fixture::new("legacy");
    f.case("tests/r/basics/arith.R", "").mark(&OLD, "tests/r", "tests/r/basics/arith.R");
    std::fs::write(f.root().join("tests/r/passing.txt"), "# comment\n\ntests/r/basics/arith.R\n").unwrap();
    assert_eq!(load_ratchet(f.root(), &OLD, "tests/r").unwrap().len(), 1);
    assert!(load_ratchet(f.root(), &RT, "tests/r").unwrap().is_empty(), "rt has no legacy list");
    std::fs::write(f.root().join("tests/r/passing.txt"), "tests/r/basics/ghost.R\n").unwrap();
    assert!(load_ratchet(f.root(), &OLD, "tests/r").is_err(), "a dangling legacy entry must be an error");
}

#[test]
fn feature_requirements_select_the_owning_build() {
    let f = Fixture::new("features");
    f.case("tests/r/basics/arith.R", "");
    let script = f.root().join("tests/r/basics/arith.R");
    assert!(enabled(&script, &[]));
    std::fs::write(script.with_extension("requires"), "bytecode-fidelity\n").unwrap();
    assert!(!enabled(&script, &[]));
    assert!(enabled(&script, &["bytecode-fidelity"]));
    assert!(scripts(&f.root().join("tests/r"), &[]).is_empty());
    assert_eq!(scripts(&f.root().join("tests/r"), &["bytecode-fidelity"]).len(), 1);
    // Selection must leave the fidelity fixture's ratchet intact.
    f.mark(&OLD, "tests/r", "tests/r/basics/arith.R");
    assert_eq!(load_ratchet(f.root(), &OLD, "tests/r").unwrap().len(), 1);
}

#[test]
fn feature_requirements_reject_unknown_features() {
    let f = Fixture::new("unknown-feature");
    f.case("tests/r/basics/arith.R", "");
    let script = f.root().join("tests/r/basics/arith.R");
    std::fs::write(script.with_extension("requires"), "misspelled-feature\n").unwrap();
    assert!(std::panic::catch_unwind(|| enabled(&script, &[])).is_err());
}

#[test]
fn sha256_matches_sha256sum() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let long = vec![b'a'; 1_000];
    assert_eq!(sha256_hex(&long), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
}

/// The pool is one pool across implementations: a slot the Python tools
/// hold (scripts/conf_slots.py, `fcntl.flock`) is held for the runner too
/// (std's `File::try_lock` is `flock(2)` on Linux), so a one-slot run
/// waits for it.
#[test]
fn a_slot_held_by_the_python_tools_is_held_for_the_runner() {
    let pool = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slots-cross-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pool);
    std::fs::create_dir_all(&pool).unwrap();
    std::fs::set_permissions(&pool, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    // the holder holds the only slot until its stdin ends: the test, not a
    // clock, says when it lets go
    let mut holder = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            "import sys; sys.path.insert(0, sys.argv[1]); import conf_slots\n\
             h = conf_slots.Pool(sys.argv[2], 1).take()\n\
             print('held', flush=True); sys.stdin.read()",
        )
        .arg(&scripts)
        .arg(&pool)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("python3 runs scripts/conf_slots.py");
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(holder.stdout.take().unwrap()), &mut line).unwrap();
    assert_eq!(line.trim(), "held");
    let f = rt_tree("slots-cross");
    let mut o = opts(&RT, cat());
    o.filter = Some("tests/rt/smoke".into());
    o.slots = Some((pool.clone(), 1));
    let root = f.root().to_path_buf();
    let h = std::thread::spawn(move || run(&root, &RT, &o).unwrap());
    // the order is the proof: while Python holds the slot the run does not
    // finish (a case alone takes milliseconds)
    std::thread::sleep(std::time::Duration::from_secs(3));
    let finished_while_held = h.is_finished();
    drop(holder.stdin.take());
    assert!(holder.wait().unwrap().success());
    let r = h.join().unwrap();
    assert!(!finished_while_held, "the run finished while Python held the only slot");
    assert!(r.outcomes.iter().all(|c| c.passed()));
    let _ = std::fs::remove_dir_all(&pool);
}

/// A case whose crosscheck could write neither its report nor
/// `<report>.lost` says so only on its stderr: the run fails on it, even for
/// a case the ratchet does not hold (whose failures otherwise only report).
#[test]
fn a_crosscheck_report_that_cannot_be_written_fails_the_run_ratcheted_or_not() {
    use std::os::unix::fs::PermissionsExt;
    let f = rt_tree("crosscheck-unwritten");
    let fake = f.root().join("unwritten-fake");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho 'robigo: bc crosscheck: cannot write the report /x: Permission denied' >&2\nexec cat \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut o = opts(&RT, fake);
    // filtered to a case with no marker: a plain failure would not fail the run
    o.filter = Some("tests/rt/smoke".into());
    let err = run(f.root(), &RT, &o).expect_err("an unwritten crosscheck report fails the run");
    assert!(err.contains("could not write the byte-code crosscheck's report"), "{err}");
    assert!(err.contains("tests/rt/smoke/one.R"), "{err}");
    // the same case without the line: a reported failure, not a failed run
    let r = run(f.root(), &RT, &opts_filtered(cat(), "tests/rt/smoke")).unwrap();
    assert!(r.outcomes.iter().all(|c| !c.report_unwritten));
}

fn opts_filtered(bin: PathBuf, filter: &str) -> Options {
    let mut o = opts(&RT, bin);
    o.filter = Some(filter.into());
    o
}
