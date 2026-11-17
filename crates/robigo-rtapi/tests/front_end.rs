//! The `robigo-rt` front end's environment (m2m3-plan F7b): `R_HOME` and the
//! directories under it, as GNU R's `R` script exports them.

use std::path::Path;
use std::process::Command;

/// `robigo-rt` with the environment `env` adds to one without `R_HOME`, the
/// three directories, or the runtime's own controls that change what it
/// writes or how it starts (as cli.rs's `run` drops them): rt-ci runs the
/// tests under those, and they are not the front end's to test.
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

#[test]
fn a_set_r_home_and_directories_are_kept() {
    assert_eq!(homes(&[("R_HOME", "/opt/x")]), ["/opt/x", "/opt/x/share", "/opt/x/include", "/opt/x/doc"]);
    assert_eq!(homes(&[("R_HOME", "/opt/x"), ("R_DOC_DIR", "/d")]), ["/opt/x", "/opt/x/share", "/opt/x/include", "/d"]);
}

/// An empty `R_HOME`, or an empty directory variable, is unset (R.sh.in's
/// `test -n "${R_HOME}"`; the script assigns the directories itself).
#[test]
fn an_empty_r_home_or_directory_is_unset() {
    let unset = homes(&[]);
    assert_eq!(homes(&[("R_HOME", "")]), unset);
    assert_eq!(homes(&[("R_SHARE_DIR", ""), ("R_INCLUDE_DIR", ""), ("R_DOC_DIR", "")]), unset);
    assert_eq!(
        homes(&[("R_HOME", "/opt/x"), ("R_SHARE_DIR", "")]),
        ["/opt/x", "/opt/x/share", "/opt/x/include", "/opt/x/doc"]
    );
}

/// Stage 0 reads base's source under `R_HOME`: an `R_HOME` without that
/// tree fails at start-up with GNU R's script's message, not with a file
/// error deeper in.
#[test]
fn stage0_says_a_missing_tree_is_missing() {
    let dir = std::env::temp_dir().join(format!("robigo-rt-no-home-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = front_end(&[("R_HOME", dir.as_os_str()), ("ROBIGO_RT_BOOT", std::ffi::OsStr::new("stage0"))])
        .args(["-e", "1"])
        .output()
        .expect("running robigo-rt");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("robigo-rt: R_HOME ('{}') not found: it has no library/base\n", dir.display())
    );
}
