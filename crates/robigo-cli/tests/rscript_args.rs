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

/// `Rscript --version` and `R --version` print GNU R's lines (R:
/// unix/Rscript.c:main, version.c:PrintVersion): parallelly checks the
/// version in `Rscript --version` before it starts a multisession worker.
/// robigo's home's `Rscript` passes `--rscript-front`; its `bin/R` runs the
/// binary as `bin/exec/R`. Run as itself, robigo prints its build.
#[cfg(unix)]
#[test]
fn front_ends_print_gnu_r_versions() {
    let robigo: std::path::PathBuf =
        std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), Into::into);
    let run = |exe: &std::path::Path, args: &[&str]| {
        let out = Command::new(exe).args(args).output().expect("spawn robigo");
        assert!(out.status.success(), "{}", out.status);
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(run(&robigo, &["--rscript-front", "--version"]), "Rscript (R) version 4.5.0 (2025-04-11)\n");
    let banner = "R version 4.5.0 (2025-04-11) -- \"How About a Twenty-Six\"\n\
        Copyright (C) 2025 The R Foundation for Statistical Computing\n\
        Platform: x86_64-pc-linux-gnu\n\
        \n\
        R is free software and comes with ABSOLUTELY NO WARRANTY.\n\
        You are welcome to redistribute it under the terms of the\n\
        GNU General Public License versions 2 or 3.\n\
        For more information about these matters see\n\
        https://www.gnu.org/licenses/.\n\
        \n";
    // (Rscript hands `--version` with more arguments on to R)
    assert_eq!(run(&robigo, &["--rscript-front", "--version", "-e", "1"]), banner);
    let dir = std::env::temp_dir().join(format!("robigo-front-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = dir.join("R");
    let _ = std::fs::remove_file(&r);
    std::os::unix::fs::symlink(&robigo, &r).unwrap();
    let as_r = run(&r, &["--version"]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(as_r, banner);
    assert!(run(&robigo, &["--version"]).starts_with("robigo "));
}
