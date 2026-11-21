//! L5's boot of base from its database (`ROBIGO_RT_BOOT=rdb`): GNU R's
//! `R_ReplFile` of R_HOME's `library/base/R/base`, which in the tree
//! `scripts/rt-home-db.sh` makes is `baseloader.R`, its `..lazyLoad`
//! binding base from the `base.rdb` robigo-rt wrote (makebasedb.R, booted
//! from base's source). Checked: base's closures are byte code (from the
//! database, compiled as they were written), baseloader.R's work after
//! `..lazyLoad` (the `.C_`/`.F_` symbols, the S3 methods table, the args
//! environments locked), `InitGraphics`'s `.Device`, and base's names: as
//! many as GNU R 4.6.2's (`ls(baseenv(), all.names = TRUE)` with
//! `R_DEFAULT_PACKAGES=NULL`: 1407).
//!
//! The tree is not in the repository: `ROBIGO_RT_HOME_DB=1` (rt-ci's step,
//! after `scripts/rt-home-db.sh`) says it must be there. Without the
//! variable the test says so and passes when the tree is missing.

use std::process::Command;

/// What base's boot gives today on stderr: the system profile's three
/// expressions that call internals robigo-rt lacks (Sys.info and Sys.glob,
/// E's). When they land, this is empty and the test changes with it.
const PROFILE_STOPS: &str = "Error in Sys.info() : \n  robigo: .Internal(Sys.info()) is not implemented yet\n\
Calls: Sys.setenv -> .expand_R_libs_env_var -> Sys.info\n\
Error in Sys.info() : \n  robigo: .Internal(Sys.info()) is not implemented yet\n\
Calls: Sys.setenv -> .expand_R_libs_env_var -> Sys.info\n\
Error in Sys.glob(path.expand(paths), dirmark) : \n  robigo: .Internal(Sys.glob()) is not implemented yet\n\
Calls: local ... eval -> eval -> eval -> eval -> .libPaths -> Sys.glob\n";

/// Whether the tree is made (else the test is skipped, unless
/// `ROBIGO_RT_HOME_DB` says it must be).
fn tree_made() -> bool {
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.robigo/rt-home-db");
    if !tree.join("library/base/R/base.rdb").is_file() {
        assert!(
            std::env::var_os("ROBIGO_RT_HOME_DB").is_none(),
            "ROBIGO_RT_HOME_DB is set, but {} has no base.rdb: make it with scripts/rt-home-db.sh",
            tree.display()
        );
        eprintln!("{} is not made: skipped (scripts/rt-home-db.sh makes it)", tree.display());
        return false;
    }
    true
}

/// robigo-rt booted from the tree with `args` and the extra environment
/// `env`: (stdout, stderr, status).
fn boot(args: &[&std::ffi::OsStr], env: &[(&str, &std::ffi::OsStr)]) -> (String, String, Option<i32>) {
    let home = std::env::temp_dir().join(format!("robigo-boot-db-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
        .args(args)
        .current_dir(&home)
        // a private cwd and HOME: no caller's .Renviron or .Rprofile
        .env("HOME", &home)
        .env("ROBIGO_RT_BOOT", "rdb")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LC_ALL", "C.UTF-8")
        .env_remove("R_HOME")
        .env_remove("ROBIGO_RT_CHECKOUT")
        // the profile's own controls (Common.R's .Last.sys under R_BATCH,
        // Rprofile.unix's R_TESTS file): not the boot under test
        .env_remove("R_BATCH")
        .env_remove("R_TESTS")
        .env_remove("R_ENVIRON")
        .env_remove("R_PROFILE")
        .env_remove("R_ENVIRON_USER")
        .env_remove("R_PROFILE_USER")
        // what the system Renviron and the tests' files read: the system
        // Renviron's own values, not the caller's
        .env_remove("R_BROWSER")
        .env_remove("R_PAPERSIZE")
        .env_remove("ROBIGO_T_UNSET")
        // rt-ci runs the tests under its observation modes (scripts/rt-ci.py
        // inherits the environment): the boot under them is not this test's
        .env_remove("ROBIGO_GCTORTURE")
        .env_remove("ROBIGO_GC_STATS")
        .env_remove("ROBIGO_GC_MOVE")
        .env_remove("ROBIGO_GC_PIN_REPORT")
        .env_remove("ROBIGO_FRAME_QUARANTINE")
        .env_remove("ROBIGO_FAIL_ALLOC_AT")
        .env_remove("ROBIGO_NO_INPLACE")
        .env_remove("ROBIGO_MATERIALIZE")
        .env_remove("ROBIGO_BOOT_VERBOSE")
        // the test's own, last (over the removals above)
        .envs(env.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
    let _ = std::fs::remove_dir_all(&home);
    (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned(), o.status.code())
}

#[test]
fn base_boots_from_its_database() {
    if !tree_made() {
        return;
    }
    let expr = "f <- get('paste', baseenv()); \
                cat(typeof(.Internal(bodyCode(f))), exists('.C_R_addTaskCallback'), exists('.F_dqrdc2'), \
                    length(ls(.BaseNamespaceEnv[['.__S3MethodsTable__.']], all.names = TRUE)) == nrow(.S3_methods_table), \
                    environmentIsLocked(.ArgsEnv), .Device, length(ls(baseenv(), all.names = TRUE)), \
                    exists('.Library.site', envir = globalenv(), inherits = FALSE), getOption('browser'), '\\n')";
    let (out, err, status) = boot(&["--vanilla".as_ref(), "-e".as_ref(), expr.as_ref()], &[]);
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("bytecode TRUE TRUE TRUE TRUE null device 1407 FALSE xdg-open \n", PROFILE_STOPS, Some(0))
    );
}

/// The start-up files after the system profile, as GNU R 4.6.2 reads them
/// (Rscript without --vanilla: the user's Renviron and profile read): a
/// user Renviron's terms and quotes, its invalid line's message (on stdout
/// this early), the user profile in the global environment, `.First`, and
/// `.Library.site` gone from it. Recorded on the oracle with the same files.
#[test]
fn the_start_up_files_are_read_as_gnu_r_reads_them() {
    if !tree_made() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let renviron = dir.join("renviron");
    std::fs::write(
        &renviron,
        "# comment\nROBIGO_T_A=plain\nROBIGO_T_B=\"quoted value\"\nROBIGO_T_C=${ROBIGO_T_UNSET-'default'}/x\n\
         ROBIGO_T_D=${ROBIGO_T_A:-no}\nnot a line\n",
    )
    .unwrap();
    let profile = dir.join("profile");
    std::fs::write(&profile, "cat(\"user profile\\n\")\n.First <- function() cat(\".First ran\\n\")\n").unwrap();
    let script = dir.join("script.R");
    std::fs::write(
        &script,
        "cat(Sys.getenv(c(\"ROBIGO_T_A\", \"ROBIGO_T_B\", \"ROBIGO_T_C\", \"ROBIGO_T_D\")), sep = \"|\"); cat(\"\\n\")\n\
         cat(exists(\".Library.site\", envir = globalenv(), inherits = FALSE), getOption(\"browser\"), Sys.getenv(\"R_PAPERSIZE\"), \"\\n\")\n",
    )
    .unwrap();
    let (out, err, status) = boot(
        &[script.as_os_str()],
        &[("R_ENVIRON_USER", renviron.as_os_str()), ("R_PROFILE_USER", profile.as_os_str())],
    );
    let gnur = format!(
        "\n   File {} contains invalid line(s)\n      not a line\n   They were ignored\n\n\
         user profile\n.First ran\nplain|quoted value|default/x|plain\nFALSE xdg-open letter \n",
        renviron.display()
    );
    // .First's error halts R (check_session_exit), .Last not run
    let profile2 = dir.join("profile2");
    std::fs::write(&profile2, ".First <- function() stop(\"boom\")\n").unwrap();
    // (an empty R_ENVIRON_USER reads no user Renviron: not the caller's)
    let (out2, err2, status2) =
        boot(&[script.as_os_str()], &[("R_PROFILE_USER", profile2.as_os_str()), ("R_ENVIRON_USER", "".as_ref())]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((out.as_str(), err.as_str(), status), (gnur.as_str(), PROFILE_STOPS, Some(0)));
    let halted = format!("{PROFILE_STOPS}Error in .First() : boom\nExecution halted\n");
    assert_eq!((out2.as_str(), err2.as_str(), status2), ("", halted.as_str(), Some(1)));
}

/// A user profile's errors as GNU R 4.6.2 handles them (recorded on the
/// oracle): `R_LoadProfile`'s SETJMP, not R_ReplFile's, takes them, so the
/// statements before a syntax error run, the error (with the failing
/// statement's context alone) halts the script, and with
/// `options(catch.script.errors = TRUE)` the rest of the profile is not
/// read and the script runs.
#[test]
fn a_user_profile_s_errors_end_it_as_in_gnu_r() {
    if !tree_made() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("robigo-boot-db-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("s.R");
    std::fs::write(&script, "cat(\"script ran\\n\")\n").unwrap();
    let run = |name: &str, text: &str| {
        let f = dir.join(name);
        std::fs::write(&f, text).unwrap();
        boot(&[script.as_os_str()], &[("R_PROFILE_USER", f.as_os_str()), ("R_ENVIRON_USER", "".as_ref())])
    };
    let parse = run("parse", "cat(\"before\\n\")\n)\ncat(\"after\\n\")\n");
    let error = run("error", "cat(\"one\\n\")\nstop(\"boom\")\ncat(\"continued\\n\")\n");
    let caught =
        run("caught", "options(catch.script.errors = TRUE)\ncat(\"one\\n\")\nstop(\"boom\")\ncat(\"continued\\n\")\n");
    let _ = std::fs::remove_dir_all(&dir);
    let parse_err = format!("{PROFILE_STOPS}Error: 2:1: unexpected ')'\n2: )\n    ^\nExecution halted\n");
    assert_eq!((parse.0.as_str(), parse.1.as_str(), parse.2), ("before\n", parse_err.as_str(), Some(1)));
    let error_err = format!("{PROFILE_STOPS}Error: boom\nExecution halted\n");
    assert_eq!((error.0.as_str(), error.1.as_str(), error.2), ("one\n", error_err.as_str(), Some(1)));
    let caught_err = format!("{PROFILE_STOPS}Error: boom\n");
    assert_eq!((caught.0.as_str(), caught.1.as_str(), caught.2), ("one\nscript ran\n", caught_err.as_str(), Some(0)));
}
