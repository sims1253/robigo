//! T2: code made from R objects stays current until a cell it was made
//! from is written through a setter (docs/g3.md §9.3; robigo-rtcore's code
//! floors). R code never writes a call or a formals cell in place, so
//! ordinary evaluation, in each profile and under GNU R's JIT decisions,
//! leaves the floors at zero: no code is ever made stale by robigo's own
//! writes. The writes from C, and what runs after them, are checked against
//! GNU R 4.6.2 by `tests/rt/capi/closure-mutation.R` and
//! `callback-mutation.R`.

mod common;

use common::*;
use robigo_rt::Profile;

#[test]
fn ordinary_evaluation_makes_no_tracked_write() {
    for mut s in [Session::new(Profile::Eval), Session::new(Profile::Bc), Session::jit()] {
        s.val("f <- function(x, y = 2) { z <- c(x, y); z[2] <- 3; names(z) <- c('a', 'b'); attr(z, 'u')[1] <- 1; z }");
        for _ in 0..3 {
            s.val("f(1)");
        }
        s.val("l <- list(a = list(b = 1)); l$a$b <- 2; l[['a']][['c']] <- 3; l$a$b");
        s.val("sf <- function(x, y) substitute(x + y); sf(a * 2, b)");
        s.val("oe <- function() { on.exit(record('exit')); 1 }; oe(); oe()");
        s.val("k <- function(n) if (n > 0) k(n - 1) else 0; k(10)");
        s.val("m <- function(v) switch(v, a = 1, b = , c = 3); m('b'); m('a')");
        s.val("g <- function(...) list(...); g(1, b = 2); g()");
        s.val("h <- function(x) { x <- x + 1; for (i in 1:3) x <- x + i; x }; h(1); h(2); h(3)");
        assert_eq!(s.cx().code_floors(), (0, 0));
    }
}
