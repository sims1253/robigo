//! The database tree a test binary's robigo-rt boots from. robigo-rt boots
//! rdb by default, from a checkout's `.robigo/rt-home-db`, which
//! `scripts/rt-home-db.sh` writes for one binary (its stamp and translation
//! cache are that binary's). A test's binary is never given the checkout's
//! tree, which the checkout's scripts and rt-ci's plans write for the
//! release binary: `scripts/rt-test-tree.sh` makes it one of its own under
//! `.robigo/test-trees/`, shaped as a checkout, and the spawned robigo-rt
//! is pointed at it (`ROBIGO_RT_CHECKOUT`). A test spawns robigo-rt through
//! [`rt_command`] (cargo's binary) or [`command_for`] (another), never
//! `Command::new` of the binary; a test whose every spawn names
//! `ROBIGO_RT_BOOT=m1` or `stage0` needs neither.
//!
//! Included with `#[path = "common/db_tree.rs"] mod db_tree;`: the test
//! binaries that use it do not need `common`'s C API session.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// The user's start-up settings a start reads (`Rf_initialize_R`'s
/// Renviron files, `setup_Rmainloop`'s profiles, the library paths and
/// default packages the system profile takes, the history, the source
/// references option, the default save and serialization versions, the
/// translations): a spawn has none of the caller's, so a developer's
/// `~/.Rprofile` or `R_LIBS_USER` never reaches a pinned transcript. The
/// JIT's variables and robigo's modes stay: steps set those on purpose.
const USER_VARS: &[&str] = &[
    "R_HOME",
    "R_ENVIRON",
    "R_ENVIRON_USER",
    "R_PROFILE",
    "R_PROFILE_USER",
    "R_LIBS",
    "R_LIBS_USER",
    "R_LIBS_SITE",
    "R_DEFAULT_PACKAGES",
    "R_HISTFILE",
    "R_HISTSIZE",
    "R_KEEP_PKG_SOURCE",
    "R_DEFAULT_SAVE_VERSION",
    "R_DEFAULT_SERIALIZE_VERSION",
    "R_TRANSLATIONS",
    "R_BATCH",
    "R_TESTS",
];

/// `Command::new(bin)` pointed at `bin`'s test tree ([`ensure_db_tree`]),
/// made first if it is missing or stale, with a private `HOME` (one empty
/// directory per test process) and none of the caller's [`USER_VARS`]. A
/// test's own `.env(...)` after it still wins: a test of the user's files
/// sets `HOME` or `R_PROFILE_USER` back itself.
pub fn command_for(bin: &Path) -> Command {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    let home = HOME.get_or_init(|| {
        let h = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rt-test-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap_or_else(|e| panic!("making {}: {e}", h.display()));
        h
    });
    let checkout = ensure_db_tree(bin);
    let mut cmd = Command::new(bin);
    cmd.env("ROBIGO_RT_CHECKOUT", checkout).env("HOME", home);
    for v in USER_VARS {
        cmd.env_remove(v);
    }
    // GNU R's sources (M1's loader reads them, base.rs's source_dir) stay
    // where the caller's HOME has them, which the private HOME would hide
    if std::env::var_os("ROBIGO_RSRC").is_none() && std::env::var_os("ROBIGO_RESEARCH_CACHE").is_none() {
        if let Some(h) = std::env::var_os("HOME") {
            cmd.env("ROBIGO_RESEARCH_CACHE", Path::new(&h).join(".cache/robigo-research"));
        }
    }
    cmd
}

/// The system profile's refusal under the default boot, first on stderr (INTERIM(srcref): E's PR deletes it).
pub const SRCREF_LINE: &str =
    "Error in options(...) : \n  robigo: options(keep.source = TRUE) (source references) is not implemented yet\n";

/// [`command_for`] of cargo's robigo-rt for this test binary.
pub fn rt_command() -> Command {
    command_for(Path::new(env!("CARGO_BIN_EXE_robigo-rt")))
}

/// The checkout-shaped directory of `bin`'s test tree
/// (`<dir>/.robigo/rt-home-db`), made once per test process and binary by
/// `scripts/rt-test-tree.sh --robigo-rt BIN`: it does nothing when the
/// tree's stamp says it is fresh, and rt-home-db.sh holds its lock while it
/// writes, so test binaries that start at once wait for one build. The
/// script runs without the torture and mode variables a step may set for
/// its tests (`ROBIGO_*` but `ROBIGO_RESEARCH_CACHE`, `R_GCTORTURE*`) and
/// without the user's [`USER_VARS`]: the tree is built as the scripts build
/// it. A failure panics with the
/// script's stderr and the command to run; no test skips or runs on another
/// boot instead.
pub fn ensure_db_tree(bin: &Path) -> PathBuf {
    static DONE: Mutex<Option<HashMap<PathBuf, Result<PathBuf, String>>>> = Mutex::new(None);
    let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
    let r = done.get_or_insert_with(HashMap::new).entry(bin.to_path_buf()).or_insert_with(|| make(bin));
    match r {
        Ok(dir) => dir.clone(),
        Err(e) => panic!("{e}"),
    }
}

fn make(bin: &Path) -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = "scripts/rt-test-tree.sh";
    let mut cmd = Command::new(root.join(script));
    cmd.arg("--robigo-rt").arg(bin).current_dir(&root);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        // (ROBIGO_RESEARCH_CACHE is where rt-home.sh finds GNU R's
        // sources: kept)
        if (k.starts_with("ROBIGO_") && k != "ROBIGO_RESEARCH_CACHE") || k.starts_with("R_GCTORTURE") {
            cmd.env_remove(&*k);
        }
    }
    // nor the user's start-up settings (R_KEEP_PKG_SOURCE=yes, say, would
    // have the compiler's database built from sources it cannot keep):
    // HOME stays, where rt-home.sh finds GNU R's sources and cargo its own
    for v in USER_VARS {
        cmd.env_remove(v);
    }
    let o = cmd.output().map_err(|e| format!("running {script}: {e}"))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        let tail: String = err.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!(
            "{script} failed ({}): the database tree for {} could not be made (run `{script} --robigo-rt {}` \
             to see why)\n{tail}",
            o.status,
            bin.display(),
            bin.display()
        ));
    }
    let dir = String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or_default().to_string();
    if dir.is_empty() {
        return Err(format!("{script} --robigo-rt {} printed no directory", bin.display()));
    }
    Ok(PathBuf::from(dir))
}
