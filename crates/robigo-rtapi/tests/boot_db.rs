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

#[test]
fn base_boots_from_its_database() {
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.robigo/rt-home-db");
    if !tree.join("library/base/R/base.rdb").is_file() {
        assert!(
            std::env::var_os("ROBIGO_RT_HOME_DB").is_none(),
            "ROBIGO_RT_HOME_DB is set, but {} has no base.rdb: make it with scripts/rt-home-db.sh",
            tree.display()
        );
        eprintln!("{} is not made: skipped (scripts/rt-home-db.sh makes it)", tree.display());
        return;
    }
    let expr = "f <- get('paste', baseenv()); \
                cat(typeof(.Internal(bodyCode(f))), exists('.C_R_addTaskCallback'), exists('.F_dqrdc2'), \
                    length(ls(.BaseNamespaceEnv[['.__S3MethodsTable__.']], all.names = TRUE)) == nrow(.S3_methods_table), \
                    environmentIsLocked(.ArgsEnv), .Device, length(ls(baseenv(), all.names = TRUE)), '\\n')";
    let o = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
        .args(["--vanilla", "-e", expr])
        .env("ROBIGO_RT_BOOT", "rdb")
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("LC_ALL", "C.UTF-8")
        .env_remove("R_HOME")
        .env_remove("ROBIGO_RT_CHECKOUT")
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
        .output()
        .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        (out.as_ref(), err.as_ref(), o.status.code()),
        ("bytecode TRUE TRUE TRUE TRUE null device 1407 \n", PROFILE_STOPS, Some(0))
    );
}
