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
