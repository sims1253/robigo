//! The `robigo-rt` front end's environment (m2m3-plan F7b): `R_HOME` and the
//! directories under it, as GNU R's `R` script exports them.

use std::path::Path;
use std::process::Command;

/// `robigo-rt` with the environment `env` adds to one without `R_HOME`, the
/// three directories, or the runtime's own controls that change what it
/// writes or how it starts: rt-ci runs the tests under those, and they are
/// not the front end's to test. (cli.rs's `run` drops fewer: it keeps
/// `ROBIGO_GC_STATS` and `ROBIGO_RT_BOOT`, filtering the `robigo gc: `
/// report lines, so that its cases hold under the inherited modes.)
fn front_end<V: AsRef<std::ffi::OsStr>>(env: &[(&str, V)]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    for v in [
        "R_HOME",
        "R_SHARE_DIR",
        "R_INCLUDE_DIR",
        "R_DOC_DIR",
        "ROBIGO_FAIL_ALLOC_AT",
        "ROBIGO_GC_STATS",
        "ROBIGO_BOOT_VERBOSE",
        "ROBIGO_RT_BOOT",
        "ROBIGO_RT_CHECKOUT",
    ] {
        cmd.env_remove(v);
    }
    // M1's loader (INTERIM(L5)): these cases test the front end's
    // environment, which every boot sets the same, and M1's reads no tree
    // (the default boot needs base's database tree)
    cmd.env("ROBIGO_RT_BOOT", "m1");
    // the pin report to stderr (`1`); to a file it stays, as rt-ci's check
    // (cli.rs's `run`)
    if std::env::var_os("ROBIGO_GC_PIN_REPORT").is_some_and(|v| v == "1") {
        cmd.env_remove("ROBIGO_GC_PIN_REPORT");
    }
    cmd.envs(env.iter().map(|(k, v)| (k, v)));
    cmd
}

/// `R.home()` and the three directories, one per line, from `robigo-rt -e`
/// under [`front_end`]'s environment.
fn homes(env: &[(&str, &str)]) -> Vec<String> {
    let mut cmd = front_end(env);
    cmd.args(["-e", r#"cat(R.home(), Sys.getenv(c("R_SHARE_DIR", "R_INCLUDE_DIR", "R_DOC_DIR")), sep = "\n")"#]);
    let out = cmd.output().expect("running robigo-rt");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect()
}

/// `p` with its `.` and `..` components resolved by name (no file system
/// access). A `..` with nothing before it to remove panics: the helper
/// compares absolute paths, and dropping it would make two different
/// paths equal.
fn normalized(p: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // (`pop` is false at the root and on an empty path)
                assert!(out.pop(), "{p:?}: '..' with nothing to remove");
            }
            c => out.push(c),
        }
    }
    out
}

#[test]
fn r_home_defaults_to_the_checkout_s_tree() {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.robigo/rt-home");
    let got = homes(&[]);
    assert!(Path::new(&got[0]).is_absolute(), "{got:?}");
    // compared lexically: the tree need not exist (a clean checkout)
    assert_eq!(normalized(Path::new(&got[0])), normalized(&std::path::absolute(tree).unwrap()), "{got:?}");
    for (i, d) in ["share", "include", "doc"].iter().enumerate() {
        assert_eq!(got[i + 1], format!("{}/{d}", got[0]));
    }
}

/// An `R_HOME` the environment has that is not robigo-rt's own (GNU R's,
/// say) is ignored, with GNU R's warning on stdout (R.sh.in's `echo`); one
/// that is its own, or an empty one, says nothing.
#[test]
fn an_inherited_r_home_is_ignored_with_gnu_rs_warning() {
    let own = homes(&[]);
    let got = homes(&[("R_HOME", "/opt/x")]);
    assert_eq!(got[0], "WARNING: ignoring environment value of R_HOME");
    assert_eq!(&got[1..], &own[..]);
    assert_eq!(homes(&[("R_HOME", own[0].as_str())]), own);
    assert_eq!(homes(&[("R_HOME", "")]), own);
}

/// The three directories are always the tree's (the script assigns them),
/// set or empty in the environment.
#[test]
fn the_directories_are_the_tree_s() {
    let own = homes(&[]);
    assert_eq!(homes(&[("R_DOC_DIR", "/d"), ("R_SHARE_DIR", ""), ("R_INCLUDE_DIR", "/i")]), own);
}

/// `ROBIGO_RT_BOOT=m1` boots M1's loader (INTERIM(L5)), which reads no
/// tree and loads no compiler at the start: GNU R's boot, the default,
/// loads the compiler when the JIT is on (R_init_jit_enabled), so
/// `loadedNamespaces()` tells the two apart.
#[test]
fn m1_boots_m1s_loader() {
    let o = front_end(&[("ROBIGO_RT_BOOT", "m1"), ("R_ENABLE_JIT", "3")])
        .args(["-e", "cat(loadedNamespaces(), '\\n')"])
        .output()
        .expect("running robigo-rt");
    assert_eq!(
        (String::from_utf8_lossy(&o.stdout).as_ref(), String::from_utf8_lossy(&o.stderr).as_ref(), o.status.code()),
        ("base \n", "", Some(0))
    );
}

/// Stage 0 reads base's source under robigo-rt's own tree: with none in
/// the default place (no install's, and a checkout, `ROBIGO_RT_CHECKOUT`,
/// whose tree is not made), it fails at start-up with GNU R's script's
/// message and the way to make the tree, not with a file error deeper in.
#[test]
fn stage0_says_a_missing_tree_is_missing() {
    let dir = std::env::temp_dir().join(format!("robigo-rt-no-home-{}", std::process::id()));
    // (a stale directory of a reused pid might hold a tree)
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = front_end(&[("ROBIGO_RT_CHECKOUT", dir.as_os_str()), ("ROBIGO_RT_BOOT", std::ffi::OsStr::new("stage0"))])
        .args(["-e", "1"])
        .output()
        .expect("running robigo-rt");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "robigo-rt: R_HOME ('{}') not found: no installed tree (<prefix>/lib/robigo) has library/base/R/base, \
             and the checkout's is not made (scripts/rt-home.sh)\n",
            dir.join(".robigo/rt-home").display()
        )
    );
}

/// The default boot (ROBIGO_RT_BOOT unset or empty: base from its
/// database) without a tree says so at start-up as stage 0 does, naming
/// the database tree's script.
#[test]
fn the_default_boot_says_a_missing_tree_is_missing() {
    let dir = std::env::temp_dir().join(format!("robigo-rt-no-db-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cmd = front_end(&[("ROBIGO_RT_CHECKOUT", dir.as_os_str())]);
    cmd.env_remove("ROBIGO_RT_BOOT");
    let out = cmd.args(["-e", "1"]).output().expect("running robigo-rt");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "robigo-rt: R_HOME ('{}') not found: no installed tree (<prefix>/lib/robigo) has library/base/R/base, \
             and the checkout's is not made (scripts/rt-home-db.sh)\n",
            dir.join(".robigo/rt-home-db").display()
        )
    );
}

#[test]
fn a_panic_after_quit_s_context_reset_aborts_without_returning() {
    use std::os::unix::process::ExitStatusExt;
    // quit()'s R_CleanUp after R_dot_Last's context reset runs under a
    // guard: a panic there (ROBIGO_PANIC_AFTER_RESET=1 makes one) ends the
    // process with abort, never unwinding into the frames the reset
    // abandoned: f()'s on.exit does not run
    let mut cmd = front_end(&[("ROBIGO_PANIC_AFTER_RESET", "1")]);
    cmd.args(["-e", "f <- function() { on.exit(cat('on.exit\\n')); quit('no', 3) }; f()"]);
    let out = cmd.output().expect("running robigo-rt");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.signal(), Some(libc::SIGABRT), "{:?} {err}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert!(err.contains("robigo: a panic after quit()'s context reset: aborting"), "{err}");
}

// (/dev/full is Linux's: the workspace's macOS runs, platforms.yml, would
// not have it; robigo-heap's tests gate their Linux-only checks the same way)
#[cfg(target_os = "linux")]
#[test]
fn a_panic_after_quit_s_context_reset_aborts_with_stderr_unwritable() {
    use std::os::unix::process::ExitStatusExt;
    // the same with stderr on /dev/full: the guard's own report cannot be
    // written, and still nothing unwinds (f()'s on.exit writes to stdout)
    let mut cmd = front_end(&[("ROBIGO_PANIC_AFTER_RESET", "1")]);
    cmd.args(["-e", "f <- function() { on.exit(cat('on.exit\\n')); quit('no', 3) }; f()"]);
    cmd.stderr(std::fs::File::options().write(true).open("/dev/full").expect("opening /dev/full"));
    let out = cmd.output().expect("running robigo-rt");
    assert_eq!(out.status.signal(), Some(libc::SIGABRT), "{:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

/// `ROBIGO_TORTURE_AFTER_START=1`, the test hook that holds torture until
/// the start is over: refused (exit 2) with no torture mode to hold, and
/// gctorture read as the heap reads it (atoi: `1suffix` is a gap of 1), so
/// the check agrees with what the heap will do.
#[test]
fn the_torture_hold_needs_a_mode_and_reads_gctorture_as_the_heap_does() {
    let modes = ["R_GCTORTURE", "R_GCTORTURE_WAIT", "ROBIGO_GCTORTURE", "ROBIGO_GC_MOVE", "ROBIGO_RETRY_TORTURE"];
    let run = |env: &[(&str, &str)]| {
        let mut cmd = front_end(env);
        for m in modes {
            if !env.iter().any(|(k, _)| *k == m) {
                cmd.env_remove(m);
            }
        }
        cmd.args(["--vanilla", "-e", "cat(gctorture(), \"\\n\")"]).output().unwrap()
    };
    let refused = run(&[("ROBIGO_TORTURE_AFTER_START", "1")]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no torture mode is set"));
    // held through the start, armed for the script: gctorture is on there
    let held = run(&[("ROBIGO_TORTURE_AFTER_START", "1"), ("R_GCTORTURE", "1suffix")]);
    assert!(held.status.success(), "{}", String::from_utf8_lossy(&held.stderr));
    assert_eq!(String::from_utf8_lossy(&held.stdout).trim(), "TRUE");
    // and a value the heap reads as off is no mode
    let off = run(&[("ROBIGO_TORTURE_AFTER_START", "1"), ("R_GCTORTURE", "0x5")]);
    assert_eq!(off.status.code(), Some(2));
}
