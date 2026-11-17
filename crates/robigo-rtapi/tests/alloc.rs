//! `for (i in 1:n)` allocates nothing that grows with `n` (AC1): `:`
//! makes a compact sequence (altclasses.c's `R_compact_intrange`), and the
//! loop reads each element from it, as GNU R's `for` does through
//! `INTEGER_ELT`. The heap's count (`ROBIGO_GC_STATS`) is the same for
//! `n` = 10 and `n` = 10^6, in the AST and in byte code, and so are those
//! of `seq_len(n)` and `seq_along(x)` over a compact `x`.

use std::process::Command;

/// The objects and bytes robigo-rt's heap allocated running `src`, from
/// `ROBIGO_GC_STATS`' report.
fn allocated(src: &str, profile: Option<&str>) -> (u64, u64) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_robigo-rt"));
    cmd.args(["-e", src]).env("LANGUAGE", "en").env("LC_ALL", "C.UTF-8").env("R_DEFAULT_PACKAGES", "NULL");
    // (rt-ci's `check` counts too, and panics on what it checks)
    if std::env::var("ROBIGO_GC_STATS").as_deref() != Ok("check") {
        cmd.env("ROBIGO_GC_STATS", "1");
    }
    match profile {
        Some(p) => cmd.env("ROBIGO_RT_PROFILE", p),
        None => cmd.env_remove("ROBIGO_RT_PROFILE"),
    };
    let o = cmd.output().expect("run robigo-rt");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "robigo-rt failed on {src:?}: {err}");
    let line = err
        .lines()
        .find_map(|l| l.strip_prefix("robigo gc: allocated "))
        .unwrap_or_else(|| panic!("no allocation count in {err}"));
    // "N objects, B bytes"
    let mut w = line.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
    let objects = w.next().unwrap().parse().unwrap();
    let bytes = w.next().unwrap().parse().unwrap();
    (objects, bytes)
}

#[test]
fn a_for_loop_over_a_compact_sequence_allocates_nothing_per_element() {
    let loops = [
        "f <- function(n) { for (i in 1:n) NULL; i }",
        "f <- function(n) { for (i in seq_len(n)) if (i < 0L) break; i }",
        "f <- function(n) { x <- 1:n; for (i in seq_along(x)) NULL; i }",
        "f <- function(n) { for (i in n:1) NULL; i }",
    ];
    for def in loops {
        for profile in [Some("eval"), Some("bc"), None] {
            let run = |n: u32| allocated(&format!("{def}; invisible(f(3)); x <- f({n})"), profile);
            let (small, large) = (run(10), run(1_000_000));
            assert_eq!(small, large, "{def} ({profile:?}): n = 10 allocates {small:?}, n = 1e6 {large:?}");
        }
    }
}
