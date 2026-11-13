//! Cost probes (ignored): run under callgrind with `ROBIGO_COST_N` = N and
//! 2N, the difference over N is the cost of one iteration (the method of
//! docs/ledger/perf-vm.md). `ROBIGO_COST_CASE` picks the workload,
//! `ROBIGO_RT_PROFILE` the profile.
//!
//! ```text
//! valgrind --tool=callgrind --callgrind-out-file=/dev/null \
//!   target/release/deps/cost-* --ignored --exact cost --nocapture
//! ```

mod common;

use common::*;
use robigo_rt::Profile;

#[test]
#[ignore]
fn cost() {
    let n: usize = std::env::var("ROBIGO_COST_N").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let case = std::env::var("ROBIGO_COST_CASE").unwrap_or_else(|_| "fib".into());
    let profile = match std::env::var("ROBIGO_RT_PROFILE").as_deref() {
        Ok("bc") => Profile::Bc,
        _ => Profile::Eval,
    };
    let mut r = Session::new(profile);
    let (setup, body) = match case.as_str() {
        "fib" => ("fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)", "fib(18)"),
        "plain" => (
            "s <- 0; tag <- function(x, label) { if (label == \"bc\") x * 2 else x }",
            "s <- s + tag(i * 0.25, \"bc\")",
        ),
        "defaults" => (
            "s <- 0; scale_by <- function(x, scale = 1, label = \"a\") { if (label == \"zz\") x else x * scale }",
            "s <- s + scale_by(i * 0.5)",
        ),
        "named" => ("s <- 0; weigh <- function(x, y) { z <- x * y; z + 1 }", "s <- s + weigh(y = 2, x = i)"),
        other => panic!("unknown case {other}"),
    };
    r.val(setup);
    let src = format!("g <- function() {{ for (i in 1:{n}) {{ {body} }}; NULL }}; g()");
    r.val(&src);
}
