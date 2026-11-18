//! L4's makeLazy and lazyLoadDBfetch through base's own `lazyLoad`
//! (lazyLoadDBexec, then `.Internal(makeLazy(...))`), on GNU R 4.6.2's
//! own lazy-load databases: for base, compiler and tools, the variables
//! bound are exactly those the `.rdx` names, and every one forces to a
//! value.
//!
//! The databases are not in the repository: `ROBIGO_ORACLE_LIBRARY` names
//! the copy `scripts/rt-rdb-roundtrip.sh` makes (CI's rt job restores it
//! from its rdb-data job). Without the variable the test says so and
//! passes.
//!
//! The values agree with GNU R's own `lazyLoad` of the same files
//! (compared object by object with serialize checksums when this was
//! written), closures' environments aside: GNU R's are the package's
//! namespace, robigo-rt's `R_GlobalEnv` (base's `..getNamespace` falls
//! back to it while the namespace cannot load from source yet).

use std::process::Command;

#[test]
fn oracle_databases_lazy_load() {
    let Some(lib) = std::env::var_os("ROBIGO_ORACLE_LIBRARY") else {
        eprintln!("ROBIGO_ORACLE_LIBRARY is not set: skipped (scripts/rt-rdb-roundtrip.sh sets it)");
        return;
    };
    let lib = std::path::PathBuf::from(lib);
    for pkg in ["base", "compiler", "tools"] {
        let file = lib.join(pkg).join("R").join(pkg);
        let f = file.display();
        let expr = format!(
            "e <- new.env(); lazyLoad('{f}', envir = e); \
             map <- .Internal(unserializeFromConn(gzcon(file('{f}.rdx', 'rb')), NULL)); \
             v <- names(map$variables); \
             ok <- vapply(v, function(n) {{ get(n, envir = e); TRUE }}, TRUE); \
             cat(length(v), setequal(ls(e, all.names = TRUE), v), all(ok), '\\n')"
        );
        let o = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
            .args(["--vanilla", "-e", &expr])
            .env("LC_ALL", "C")
            .env("R_DEFAULT_PACKAGES", "NULL")
            .env_remove("R_HOME")
            .env_remove("ROBIGO_RT_CHECKOUT")
            .output()
            .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
        let out = String::from_utf8_lossy(&o.stdout);
        eprintln!("{pkg}: {}", out.trim().lines().last().unwrap_or(""));
        assert!(
            o.status.success() && out.trim_end().ends_with("TRUE TRUE"),
            "{pkg}: lazyLoad bound or forced wrongly: {out}{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
