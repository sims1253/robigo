//! L4's makeLazy and lazyLoadDBfetch through base's own `lazyLoad`
//! (lazyLoadDBexec, then `.Internal(makeLazy(...))`), on GNU R 4.6.2's own
//! lazy-load databases of base, compiler and tools, against GNU R's own
//! `lazyLoad` of the same files:
//! - every variable the `.rdx` names is bound, and only those, and there
//!   are as many as the oracle's databases hold (a truncated or empty map
//!   fails);
//! - each value forces, and its serialization agrees with GNU R's
//!   (`PKG.lazyload`, written beside the database by GNU R in the oracle
//!   image: a checksum per variable, and one with every closure in the
//!   value replaced by its formals, body and attributes and every
//!   environment by its sorted contents).
//!
//! One class of difference is expected, and only while its reason holds:
//! **closures' environments** (and environments' enclosures). GNU R's
//! closures of compiler and tools have the package's namespace as
//! environment; robigo-rt's have `R_GlobalEnv`,
//! because base's `..getNamespace` falls back to it while the namespace
//! cannot load from source (`loadNamespace(PKG)` fails). A value whose
//! checksum differs but whose closures-aside checksum agrees is that class
//! when the namespace does not load; once it loads, the class is no longer
//! allowed and such a value fails.
//!
//! The databases and GNU R's checksums are not in the repository:
//! `ROBIGO_ORACLE_LIBRARY` names the copy `scripts/rt-rdb-roundtrip.sh`
//! makes (CI's rt job restores it from its rdb-data job). Without the
//! variable the test says so and passes; with it set and a file missing, it
//! fails.

use std::process::Command;

/// The packages and how many variables their oracle databases hold.
const PACKAGES: [(&str, usize); 3] = [("base", 1179), ("compiler", 289), ("tools", 915)];

/// robigo-rt's side, as rt-rdb-roundtrip.sh's GNU R side computes it: the
/// database (RT_LAZYLOAD_FILE, without its extension) lazy-loaded, then per
/// variable in sorted order its name and the two checksums; then whether
/// the package's namespace loads.
const SCRIPT: &str = r#"
f <- Sys.getenv("RT_LAZYLOAD_FILE")
pkg <- Sys.getenv("RT_LAZYLOAD_PKG")
ck <- function(x) { b <- as.numeric(as.integer(serialize(x, NULL, version = 2L))[-(1:14)])
                    paste(length(b), sum(b * seq_along(b)) %% 1000000007) }
nz <- function(x, top = TRUE) if (is.function(x) && !is.primitive(x)) list(formals(x), body(x), attributes(x)) else
                  if (is.environment(x)) { if (top) list("env", nz(as.list(x, all.names = TRUE, sorted = TRUE), FALSE)) else "env" } else
                  if (is.list(x)) { a <- attributes(x); r <- lapply(x, nz, top = top); attributes(r) <- a; r } else x
e <- new.env()
invisible(lazyLoad(f, envir = e))
map <- .Internal(unserializeFromConn(gzcon(file(paste0(f, ".rdx"), "rb")), NULL))
v <- names(map$variables)
cat("variables", length(v), setequal(ls(e, all.names = TRUE), v), "\n")
for (n in sort(ls(e, all.names = TRUE))) { x <- get(n, envir = e); cat(n, ck(x), ck(nz(x)), "\n") }
cat("loadable", pkg == "base" || tryCatch({ loadNamespace(pkg); TRUE }, error = function(e) FALSE), "\n")
"#;

#[test]
fn oracle_databases_lazy_load() {
    let Some(lib) = std::env::var_os("ROBIGO_ORACLE_LIBRARY") else {
        eprintln!("ROBIGO_ORACLE_LIBRARY is not set: skipped (scripts/rt-rdb-roundtrip.sh sets it)");
        return;
    };
    let lib = std::path::PathBuf::from(lib);
    // set but incomplete is a broken setup, never a skip
    for (pkg, _) in PACKAGES {
        for ext in ["rdb", "rdx", "lazyload"] {
            let f = lib.join(pkg).join("R").join(format!("{pkg}.{ext}"));
            assert!(
                f.is_file(),
                "ROBIGO_ORACLE_LIBRARY={} is set, but {} is missing: make the data with \
                 scripts/rt-rdb-roundtrip.sh --data-only {}",
                lib.display(),
                f.display(),
                lib.display()
            );
        }
    }
    let mut failures = Vec::new();
    for (pkg, count) in PACKAGES {
        let file = lib.join(pkg).join("R").join(pkg);
        let o = Command::new(env!("CARGO_BIN_EXE_robigo-rt"))
            .args(["--vanilla", "-e", SCRIPT])
            .env("RT_LAZYLOAD_FILE", &file)
            .env("RT_LAZYLOAD_PKG", pkg)
            .env("LC_ALL", "C")
            .env("R_DEFAULT_PACKAGES", "NULL")
            .env_remove("R_HOME")
            // rt-ci runs the tests under its observation modes (scripts/rt-ci.py
            // inherits the environment): this oracle run is not their subject
            .env_remove("ROBIGO_GCTORTURE")
            .env_remove("ROBIGO_GC_STATS")
            .env_remove("ROBIGO_GC_MOVE")
            .env_remove("ROBIGO_GC_PIN_REPORT")
            .env_remove("ROBIGO_FRAME_QUARANTINE")
            .env_remove("ROBIGO_FAIL_ALLOC_AT")
            .env_remove("ROBIGO_RT_BOOT")
            .output()
            .unwrap_or_else(|e| panic!("running robigo-rt: {e}"));
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(o.status.success(), "{pkg}: robigo-rt failed: {out}{}", String::from_utf8_lossy(&o.stderr));
        let lines: Vec<Vec<&str>> = out.lines().map(|l| l.split_whitespace().collect()).collect();
        let head = lines.iter().find(|l| l.first() == Some(&"variables")).unwrap_or_else(|| {
            panic!(
                "the variables line (robigo-rt printed stdout:\n{out}\nstderr:\n{})",
                String::from_utf8_lossy(&o.stderr)
            )
        });
        assert_eq!(
            (head[1].parse::<usize>().ok(), head[2]),
            (Some(count), "TRUE"),
            "{pkg}: the .rdx's variables and the bindings lazyLoad made"
        );
        let loadable = lines.iter().any(|l| l.as_slice() == ["loadable", "TRUE"]);
        let ours: std::collections::BTreeMap<&str, (&str, &str, &str, &str)> =
            lines.iter().filter(|l| l.len() == 5).map(|l| (l[0], (l[1], l[2], l[3], l[4]))).collect();
        let gnur = std::fs::read_to_string(file.with_extension("lazyload")).unwrap();
        let (mut same, mut expected) = (0, 0);
        let mut seen = 0;
        for l in gnur.lines() {
            let g: Vec<&str> = l.split_whitespace().collect();
            let (name, full, norm) = (g[0], (g[1], g[2]), (g[3], g[4]));
            seen += 1;
            match ours.get(name) {
                None => failures.push(format!("{pkg}: {name}: not bound by robigo-rt")),
                Some(&(a, b, _, _)) if (a, b) == full => same += 1,
                // closures' environments aside, while the namespace does
                // not load from source
                Some(&(_, _, c, d)) if (c, d) == norm && full != norm && !loadable => expected += 1,
                Some(_) => failures.push(format!(
                    "{pkg}: {name}: differs from GNU R's lazyLoad{}",
                    if loadable { " (its namespace loads: closures' environments must agree)" } else { "" }
                )),
            }
        }
        assert_eq!(seen, count, "{pkg}: GNU R's {pkg}.lazyload has {seen} variables, not {count}");
        eprintln!("{pkg}: {same} the same, {expected} closures' environments aside (namespace loads: {loadable})");
    }
    assert!(failures.is_empty(), "{} variable(s) differ:\n{}", failures.len(), failures.join("\n"));
}
