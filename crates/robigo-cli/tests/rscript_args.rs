//! `Rscript`'s command line, which the conformance runner has no way to
//! vary.

use std::process::Command;

/// R: unix/Rscript.c -- what follows the last `-e` goes to the script: how
/// a PSOCK worker gets its `MASTER=` and `PORT=`. Compared with GNU R 4.5.0.
#[test]
fn what_follows_e_goes_to_the_script() {
    // As in conformance.rs: `ROBIGO_CONF_BIN` tests the binary CI ships.
    let robigo: std::path::PathBuf =
        std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), Into::into);
    let out = Command::new(robigo)
        .args(["--vanilla", "-e", "print(commandArgs(TRUE))", "--x", "MASTER=localhost", "-y", "PORT=1"])
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LANGUAGE", "en")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("spawn robigo");
    assert!(out.status.success(), "{}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "[1] \"--x\"              \"MASTER=localhost\" \"-y\"               \"PORT=1\"          \n"
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}
