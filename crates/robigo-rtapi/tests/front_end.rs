//! The `robigo-rt` front end's environment (m2m3-plan F7b): `R_HOME` and the
//! directories under it, as GNU R's `R` script exports them.

use std::path::Path;
use std::process::Command;

/// `R.home()` and the three directories, one per line, from `robigo-rt -e`
/// with the environment `env` adds to one without any of them.
fn homes(env: &[(&str, &str)]) -> Vec<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    for v in ["R_HOME", "R_SHARE_DIR", "R_INCLUDE_DIR", "R_DOC_DIR"] {
        cmd.env_remove(v);
    }
    cmd.envs(env.iter().copied());
    cmd.args(["-e", r#"cat(R.home(), Sys.getenv(c("R_SHARE_DIR", "R_INCLUDE_DIR", "R_DOC_DIR")), sep = "\n")"#]);
    let out = cmd.output().expect("running robigo-rt");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect()
}

#[test]
fn r_home_defaults_to_the_checkout_s_tree() {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.robigo/rt-home");
    let tree = std::path::absolute(tree).unwrap();
    let got = homes(&[]);
    assert_eq!(Path::new(&got[0]).canonicalize().ok(), tree.canonicalize().ok(), "{got:?}");
    assert!(Path::new(&got[0]).is_absolute());
    for (i, d) in ["share", "include", "doc"].iter().enumerate() {
        assert_eq!(got[i + 1], format!("{}/{d}", got[0]));
    }
}

#[test]
fn a_set_r_home_and_directories_are_kept() {
    assert_eq!(homes(&[("R_HOME", "/opt/x")]), ["/opt/x", "/opt/x/share", "/opt/x/include", "/opt/x/doc"]);
    assert_eq!(homes(&[("R_HOME", "/opt/x"), ("R_DOC_DIR", "/d")]), ["/opt/x", "/opt/x/share", "/opt/x/include", "/d"]);
}
