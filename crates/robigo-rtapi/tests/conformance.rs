//! robigo-rt's own conformance (docs/g3.md L1, §14.3): `tests/rt/**/*.R`,
//! and the `tests/r` cases it has ratcheted, against GNU R 4.6.2's
//! recordings (`<suite>/R-4.6.2/`), with its own ratchet column
//! `<suite>/passing-rt/`. The runner is `robigo-conformance`'s column `RT`,
//! which robigo-cli's conformance test shares (column `OLD`).
//!
//!   cargo test --release -p robigo-rtapi --test conformance -- --nocapture
//!   ROBIGO_CONF_UPDATE=1 ...    add markers for the cases that pass
//!   ROBIGO_CONF_SWEEP=1 ...     also run every tests/r case, not only the
//!                               ratcheted ones
//!   ROBIGO_RT_BIN=path          test this binary instead of cargo's
//!
//! The cases with package C load it from RT_CAPI_LIBS: the test builds
//! `tests/rt/capi/support/*.c` for robigo-rt first (`scripts/rt-capi-build.sh`,
//! into the target's temporary directory).
//!
//! ROBIGO_CONF_FILTER, ROBIGO_CONF_DIFF and ROBIGO_R_VERSION are as
//! `robigo_conformance` documents. The binary variable is rt's own: CI's
//! `test` job points `ROBIGO_CONF_BIN` at the old runtime's binary for the
//! whole workspace.

use robigo_conformance as conf;
use std::path::{Path, PathBuf};

// Rebuild when another checkout built this test last (.cargo/config.toml):
// the repository root below is compiled in.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

fn rt_bin() -> PathBuf {
    std::env::var_os("ROBIGO_RT_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo-rt").into(), PathBuf::from)
}

#[test]
fn rt_conformance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    // the package C of tests/rt/capi, built for robigo-rt (the cases load it
    // from RT_CAPI_LIBS; scripts/regen-expected.sh builds GNU R's), in a
    // directory of this checkout's: checkouts share the target directory
    let mut h = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&root, &mut h);
    let libs = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rt-capi-{:016x}", std::hash::Hasher::finish(&h)));
    let st = std::process::Command::new(root.join("scripts/rt-capi-build.sh"))
        .arg(&libs)
        .status()
        .unwrap_or_else(|e| panic!("running scripts/rt-capi-build.sh: {e}"));
    assert!(st.success(), "scripts/rt-capi-build.sh {}: {st}", libs.display());
    let mut o = conf::Options::from_env(&conf::RT, rt_bin()).unwrap_or_else(|e| panic!("{e}"));
    o.env.push(("RT_CAPI_LIBS".into(), Some(libs.into())));
    // (robigo-rt's R_HOME is its own tree, which its front end finds when
    // the environment names none: the RT column drops an inherited one from
    // each case's environment, `Column::env_remove`, so the CI image's
    // /usr/lib/R and its tools.so, built for GNU R's objects, are never
    // loaded; this process's environment is left alone)
    // the cases that load tools' native code from the binary's R_HOME tree
    // run where scripts/rt-home.sh built it (CI's rt job does)
    if has_rt_home_tools(&o.bin) {
        o.features.push("r-home-tools");
    }
    conf::run(&root, &conf::RT, &o).unwrap_or_else(|e| panic!("{e}"));
}

/// Whether `bin`'s `R.home()` is a tree robigo's scripts made (its stamp:
/// scripts/rt-home.sh's `.rt-home`, or scripts/rt-home-db.sh's `.rt-home-db`
/// for the rdb boot's copy) and has tools' native code, run as the cases are
/// (with no inherited R_HOME). An installed GNU R's
/// R_HOME (the test job's `R_HOME=/usr/lib/R`) has a tools.so too, but one
/// built for GNU R's objects, which robigo-rt must not load.
fn has_rt_home_tools(bin: &Path) -> bool {
    let out = std::process::Command::new(bin)
        .env_remove("R_HOME")
        .args([
            "-e",
            "h <- R.home(); cat(any(file.exists(file.path(h, c('.rt-home', '.rt-home-db')))) && \
             file.exists(file.path(h, 'library/tools/libs/tools.so')))",
        ])
        .output()
        .unwrap_or_else(|e| panic!("running {}: {e}", bin.display()));
    out.stdout == b"TRUE"
}

/// The verification modes' variables (scripts/rt-ci.py's steps), which a
/// test that runs one case as the plain binary does clears for it
/// (scripts/test-rt-ci.py checks that every variable a crate-test step sets
/// is here or is known not to change the binary's behaviour).
const MODE_VARS: &[&str] = &[
    "R_GCTORTURE",
    "R_GCTORTURE_WAIT",
    "ROBIGO_GCTORTURE",
    "ROBIGO_GC_MOVE",
    "ROBIGO_TORTURE_AFTER_START",
    "ROBIGO_RETRY_TORTURE",
    "ROBIGO_REQUIRE_RETRY_TORTURE",
    "ROBIGO_RT_BOOT",
    "ROBIGO_FAIL_ALLOC_AT",
    "ROBIGO_NO_INPLACE",
    "ROBIGO_MATERIALIZE",
    "ROBIGO_FRAME_QUARANTINE",
    "ROBIGO_POISON_DEAD",
    "ROBIGO_GC_STATS",
    "ROBIGO_GC_PIN_REPORT",
    "ROBIGO_GC_FREEZE",
    "ROBIGO_GC_FREEZE_CHECK",
    "ROBIGO_GC_FREEZE_CHECK_REPORT",
    "R_VSIZE",
    "R_MAX_VSIZE",
];

/// The lost-record rule end to end: robigo-rt's crosscheck (its
/// `RECORD_LOST` line) and the runner (its `UNWRITTEN_REPORT`) are two
/// crates' strings; here the real binary loses a record (its report's
/// directory is a file: neither the report nor `.lost` can be made, as root
/// too) and the real runner fails the run on that case, though no marker
/// holds it, with the binary's own line.
#[test]
fn a_lost_crosscheck_record_fails_the_run() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lost-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let id = "tests/rt/lost/compiled.R";
    let script = dir.join(id);
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    // one closure with byte code: the crosscheck has one record to write
    std::fs::write(&script, "f <- compiler::cmpfun(function(x) x + 1)\ncat(f(1), \"\\n\")\n").unwrap();
    let base = conf::expected_base(&dir, "tests/rt", &script, conf::RT.gated);
    std::fs::create_dir_all(base.parent().unwrap()).unwrap();
    let sha = conf::sha256_hex(&std::fs::read(&script).unwrap());
    for (ext, body) in [("Rout", "2 \n".to_string()), ("Rerr", String::new()), ("Rstatus", "0\n".into()), ("Rsha", sha)]
    {
        std::fs::write(format!("{}.{ext}", base.display()), body).unwrap();
    }
    let not_a_dir = dir.join("a-file");
    std::fs::write(&not_a_dir, "").unwrap();
    let mut o = conf::Options::from_env(&conf::RT, rt_bin()).unwrap_or_else(|e| panic!("{e}"));
    o.filter = Some("tests/rt/lost".into());
    o.slots = None;
    o.times = None;
    // the case runs as the plain binary does, whatever mode the step that
    // runs this test is in (rt-ci runs the crates' tests under gctorture,
    // the moving collector, ...: under them the case would time out, and a
    // timeout is no lost record)
    for k in MODE_VARS {
        o.env.push((k.into(), None));
    }
    o.env.push(("R_ENABLE_JIT".into(), Some("0".into())));
    o.env.push(("ROBIGO_BC_CROSSCHECK".into(), Some("1".into())));
    o.env.push(("ROBIGO_BC_CROSSCHECK_REPORT".into(), Some(not_a_dir.join("report").into())));
    let err = match conf::run(&dir, &conf::RT, &o) {
        Err(e) => e,
        Ok(r) => panic!(
            "the run did not fail on a lost record: {:?}",
            r.outcomes.iter().map(|c| &c.diff).collect::<Vec<_>>()
        ),
    };
    let _ = std::fs::remove_dir_all(&dir);
    assert!(err.contains("lost a record of the byte-code crosscheck"), "{err}");
    assert!(err.contains(&format!("{id}: robigo: bc crosscheck: record lost: cannot write the report")), "{err}");
    assert!(err.contains("Not a directory"), "the binary's own reason: {err}");
}
