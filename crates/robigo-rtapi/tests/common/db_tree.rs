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
pub const USER_VARS: &[&str] = &[
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

/// The observation modes rt-ci's crate-test steps set (scripts/rt-ci.py
/// inherits them to every spawn): torture, the moving collector, the
/// freeze and its check, the poisons, the copies, injected failures and the
/// reports. A spawn they are not the subject of drops them all
/// ([`without_modes`]); scripts/test-rt-ci.py checks that every variable
/// those steps set is here (or is the runner's, the tree's, or a case's,
/// as conformance.rs's `CASE_VARS`). `ROBIGO_TORTURE_AFTER_START` goes with
/// the modes it holds: alone it is refused.
pub const MODE_VARS: &[&str] = &[
    "R_GCTORTURE",
    "R_GCTORTURE_WAIT",
    "ROBIGO_GCTORTURE",
    "ROBIGO_TORTURE_AFTER_START",
    "ROBIGO_RETRY_TORTURE",
    "ROBIGO_REQUIRE_RETRY_TORTURE",
    "ROBIGO_GC_MOVE",
    "ROBIGO_GC_STATS",
    "ROBIGO_GC_PIN_REPORT",
    "ROBIGO_GC_FREEZE",
    "ROBIGO_GC_FREEZE_CHECK",
    "ROBIGO_GC_FREEZE_CHECK_REPORT",
    "ROBIGO_FAIL_ALLOC_AT",
    "ROBIGO_NO_INPLACE",
    "ROBIGO_MATERIALIZE",
    "ROBIGO_FRAME_QUARANTINE",
    "ROBIGO_POISON_DEAD",
];

/// `cmd` without any of [`MODE_VARS`]: call it before the spawn's own
/// `.env(...)`, which then wins.
pub fn without_modes(cmd: &mut Command) -> &mut Command {
    for v in MODE_VARS {
        cmd.env_remove(v);
    }
    cmd
}

/// `Command::new(bin)` pointed at `bin`'s test tree ([`ensure_db_tree`]),
/// made first if it is missing or stale ([`command_in`]).
pub fn command_for(bin: &Path) -> Command {
    command_in(bin, &ensure_db_tree(bin))
}

/// `Command::new(bin)` pointed at the checkout-shaped directory `checkout`,
/// with a private `HOME` ([`private_home`]) and none of the caller's
/// [`USER_VARS`]. A test's own `.env(...)` after it still wins: a test of
/// the user's files sets `HOME` or `R_PROFILE_USER` back itself. [`command_for`] is the way
/// in; this is for a caller that has its tree already (a probe given
/// another binary).
pub fn command_in(bin: &Path, checkout: &Path) -> Command {
    let home = private_home();
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

/// This test process's `HOME`: a new, empty directory of its own, mode
/// 0700, in the system's temporary directory (or, where that takes none,
/// cargo's `CARGO_TARGET_TMPDIR`) ([`unique_dir`]), the same path for every
/// later spawn. Nothing removes it: processes of one test binary run at
/// once (two checkouts share cargo's target directory; containers share it
/// too and see other pids), and a home that is never shared and never swept
/// cannot be emptied under a running test or a robigo-rt it left behind.
/// It is one empty directory per test process; the system clears its
/// temporary directory.
pub fn private_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        // (cargo's `<name>-<hash>`: the name alone, which builds keep)
        let name = std::env::current_exe()
            .ok()
            .and_then(|e| e.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .map(|s| match s.rsplit_once('-') {
                Some((n, h)) if h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()) => n.to_string(),
                _ => s,
            })
            .unwrap_or_else(|| "test".into());
        home_in(&std::env::temp_dir(), &format!("robigo-rt-test-home-{name}"))
    })
}

/// [`unique_dir`] in `tmp`, or in cargo's `CARGO_TARGET_TMPDIR` when `tmp`
/// takes no directory ([`private_home`]'s, `tmp` the system's).
pub fn home_in(tmp: &Path, prefix: &str) -> PathBuf {
    try_unique_dir(tmp, prefix).unwrap_or_else(|_| unique_dir(Path::new(env!("CARGO_TARGET_TMPDIR")), prefix))
}

/// A new, empty `<parent>/<prefix>-<pid>-<n>`, an absolute path (a spawn
/// that runs in another directory finds it as well), made with
/// `create_dir`, which fails on a name that exists, at the first `n` whose
/// name no one has: two processes, in two containers with the same pid
/// too, never get one directory. Its mode is 0700: no more from its making,
/// so no other user writes into it first, whatever the umask, and then
/// exactly that, so a umask that takes the owner's bits leaves it usable.
/// Panics if `parent` takes none.
pub fn unique_dir(parent: &Path, prefix: &str) -> PathBuf {
    try_unique_dir(parent, prefix).unwrap_or_else(|e| panic!("making a directory in {}: {e}", parent.display()))
}

/// [`unique_dir`], with the error when `parent` takes no directory.
pub fn try_unique_dir(parent: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let parent = std::path::absolute(parent)?;
    let pid = std::process::id();
    for n in 0u32.. {
        let d = parent.join(format!("{prefix}-{pid}-{n}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&d) {
            Ok(()) => {
                std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
                return Ok(d);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// A copy of robigo-rtapi's `shell_word` (src/main.rs, tested there by
/// `a_shell_word_is_one_word`), for the messages that quote a path as a
/// command's word: as it is when every byte is one the shell leaves alone,
/// else single-quoted.
pub fn shell_word(s: &str) -> String {
    let plain = !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+,:@%=".contains(&b));
    if plain { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

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
            shell_word(&bin.to_string_lossy())
        ));
    }
    let dir = String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or_default().to_string();
    if dir.is_empty() {
        return Err(format!("{script} --robigo-rt {} printed no directory", bin.display()));
    }
    Ok(PathBuf::from(dir))
}
