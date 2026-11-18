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

/// Whether `bin`'s `R.home()` is the tree scripts/rt-home.sh made (its
/// stamp, `.rt-home`) and has tools' native code, run as the cases are
/// (with no inherited R_HOME). An installed GNU R's
/// R_HOME (the test job's `R_HOME=/usr/lib/R`) has a tools.so too, but one
/// built for GNU R's objects, which robigo-rt must not load.
fn has_rt_home_tools(bin: &Path) -> bool {
    let out = std::process::Command::new(bin)
        .env_remove("R_HOME")
        .args(["-e", "cat(all(file.exists(file.path(R.home(), c('.rt-home', 'library/tools/libs/tools.so')))))"])
        .output()
        .unwrap_or_else(|e| panic!("running {}: {e}", bin.display()));
    out.stdout == b"TRUE"
}
