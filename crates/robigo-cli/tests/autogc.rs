//! The automatic cycle collection's decisions (autogc.rs): they depend on
//! what the program allocates, never on the environment or the binary
//! (#589), which a conformance script cannot vary.

use std::process::Command;

/// How many automatic collections `expr` triggers, with `pad` bytes of an
/// extra environment variable. Startup does not read the variable, so the
/// net count must not see it: the pads pin that the environment is inert.
fn collections(expr: &str, pad: usize) -> usize {
    // As in conformance.rs: `ROBIGO_CONF_BIN` tests the binary CI ships.
    let robigo: std::path::PathBuf =
        std::env::var_os("ROBIGO_CONF_BIN").map_or_else(|| env!("CARGO_BIN_EXE_robigo").into(), Into::into);
    let out = Command::new(robigo)
        .args(["--vanilla", "-e", expr])
        .env("R_DEFAULT_PACKAGES", "NULL")
        .env("ROBIGO_CYCLES_LOG", "1")
        .env_remove("ROBIGO_CYCLES")
        .env_remove("ROBIGO_CYCLES_TORTURE")
        .env("ROBIGO_TEST_PAD", "x".repeat(pad))
        .output()
        .expect("spawn robigo");
    assert!(out.status.success(), "{}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.starts_with("[cycles]")).count()
}

/// 300 unreachable cycles of 1 MB each: one collection per `R_VSIZE`
/// (64 MB) they hold, whatever the environment.
#[test]
fn leaked_cycles_are_collected_every_r_vsize() {
    let leak = "mk <- function() { e <- new.env(); e$x <- numeric(1.25e5); e$f <- function() e; NULL }
        for (i in 1:300) mk()";
    for pad in [0, 1, 4096] {
        assert_eq!(collections(leak, pad), 4, "with {pad} bytes of padding");
    }
}

/// Temporaries that reference counting frees before the next safe point
/// never bring the heap to the threshold there: a loop of 8 MB vectors
/// over an 8 MB `x` does not collect.
#[test]
fn freed_temporaries_do_not_collect() {
    let churn = "x <- sin(1:1e6)
        for (k in 1:5) { pos <- x[x > 0]; y <- ifelse(x > 0.5, x, -x); s <- cumsum(x); o <- order(x) }";
    for pad in [0, 1, 4096] {
        assert_eq!(collections(churn, pad), 0, "with {pad} bytes of padding");
    }
}

/// A temporary larger than the whole threshold (96 MB against `R_VSIZE`'s
/// 64 MB), allocated and freed again in every iteration, is never held at
/// a safe point: no collection.
#[test]
fn huge_freed_temporaries_do_not_collect() {
    let churn = "s <- 0
        for (i in 1:20) { s <- s + sum(numeric(1.2e7)); v <- numeric(1.2e7); v <- NULL }";
    for pad in [0, 1, 4096] {
        assert_eq!(collections(churn, pad), 0, "with {pad} bytes of padding");
    }
}

/// A 96 MB vector replaced in every iteration is held across the loop: one
/// collection, after which the heap is sized for it. The replaced vector
/// is freed at the assignment, as in GNU R, even where the bytecode defers
/// the write to the back edge's check.
#[test]
fn a_held_vector_collects_once() {
    for expr in ["for (i in 1:20) v <- numeric(1.2e7)", "f <- function() for (i in 1:20) v <- numeric(1.2e7); f()"] {
        for pad in [0, 1, 4096] {
            assert_eq!(collections(expr, pad), 1, "{expr} with {pad} bytes of padding");
        }
    }
}

/// A list that keeps growing by 8 MB vectors (480 MB in all): a collection
/// each time it fills the heap, which grows by GNU R's `AdjustHeapSize`
/// steps (64, 101, 138, 184, ... MB above the session's start).
#[test]
fn retained_growth_collects_as_the_heap_grows() {
    let grow = "l <- list(); for (i in 1:60) l[[i]] <- numeric(1e6)";
    for pad in [0, 1, 4096] {
        assert_eq!(collections(grow, pad), 7, "with {pad} bytes of padding");
    }
}
