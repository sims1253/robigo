//! V1b: subsetting and its setters, complex assignment, `switch`,
//! `substitute` and `on.exit`, in both profiles. Every expected value and
//! message is GNU R 4.6.2's (the oracle with the JIT off).

mod common;

use common::*;
use robigo_rt::Profile;

fn sessions() -> [Session; 2] {
    [Session::new(Profile::Eval), Session::new(Profile::Bc)]
}

fn reals(xs: &[f64]) -> Val {
    Val::Real(xs.to_vec())
}

fn ints(xs: &[i32]) -> Val {
    Val::Int(xs.to_vec())
}

fn strs(xs: &[&str]) -> Val {
    Val::Str(xs.iter().map(|s| s.to_string()).collect())
}

const NA_REAL_BITS: u64 = 0x7FF00000000007A2;

fn na_real() -> f64 {
    f64::from_bits(NA_REAL_BITS)
}

/// Doubles compared bit for bit (NA is a NaN payload).
fn same(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Real(x), Val::Real(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits() || (p == q))
        }
        (Val::List(x), Val::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

fn check(s: &mut Session, src: &str, want: Val) {
    let got = s.val(src);
    assert!(same(&got, &want), "{src}\n got {got:?}\nwant {want:?}");
}

#[test]
fn subsetting_vectors() {
    for mut s in sessions() {
        s.val("x <- c(10, 20, 30)");
        check(&mut s, "x[2]", reals(&[20.0]));
        check(&mut s, "x[c(3, 1)]", reals(&[30.0, 10.0]));
        check(&mut s, "x[-1]", reals(&[20.0, 30.0]));
        check(&mut s, "x[c(TRUE, FALSE)]", reals(&[10.0, 30.0]));
        check(&mut s, "x[5]", reals(&[na_real()]));
        check(&mut s, "x[0]", reals(&[]));
        check(&mut s, "x[]", reals(&[10.0, 20.0, 30.0]));
        s.val("y <- c(a = 1L, b = 2L, c = 3L)");
        check(&mut s, "y['b']", ints(&[2]));
        check(&mut s, "names(y['b'])", strs(&["b"]));
        check(&mut s, "y[c('c', 'z')]", ints(&[3, NA]));
        check(&mut s, "names(y[c('c', 'z')])", strs(&["c", "NA"]));
        check(&mut s, "y[[2]]", ints(&[2]));
        check(&mut s, "y[['c']]", ints(&[3]));
        check(&mut s, "names(y[[3]])", Val::Null);
    }
}

#[test]
fn subsetting_lists_and_environments() {
    for mut s in sessions() {
        s.val("l <- list(a = 1, b = 'x', c = list(d = 4))");
        check(&mut s, "l$a", reals(&[1.0]));
        check(&mut s, "l[['c']]$d", reals(&[4.0]));
        check(&mut s, "l[['c']][['d']]", reals(&[4.0]));
        check(&mut s, "l$zz", Val::Null);
        // no partial match: 'a' is no completion of 'ab'
        check(&mut s, "l$ab", Val::Null);
        check(&mut s, "l[c('b', 'a')]", Val::List(vec![str1("x"), real(1.0)]));
        check(&mut s, "names(l[c('b', 'a')])", strs(&["b", "a"]));
        check(&mut s, "l[[c(3, 1)]]", reals(&[4.0]));
        s.val("e <- new.env(); e$a <- 1; e[['b']] <- 2");
        check(&mut s, "e$a + e[['b']]", reals(&[3.0]));
        check(&mut s, "e$zz", Val::Null);
    }
}

#[test]
fn subassignment() {
    for mut s in sessions() {
        check(&mut s, "z <- 1:3; z[2] <- 10L; z", ints(&[1, 10, 3]));
        check(&mut s, "z[5] <- 7L; z", ints(&[1, 10, 3, NA, 7]));
        check(&mut s, "z[2] <- 2.5; z", reals(&[1.0, 2.5, 3.0, na_real(), 7.0]));
        check(&mut s, "w <- c(1, 2, 3); w[[2]] <- 9; w", reals(&[1.0, 9.0, 3.0]));
        check(&mut s, "w[c(1, 3)] <- c(-1, -3); w", reals(&[-1.0, 9.0, -3.0]));
        check(&mut s, "w[c(FALSE, TRUE, FALSE)] <- 0; w", reals(&[-1.0, 0.0, -3.0]));
        check(&mut s, "v <- list(); v$p <- 1; v[['q']] <- 'two'; v$p <- NULL; v", Val::List(vec![str1("two")]));
        check(&mut s, "names(v)", strs(&["q"]));
        check(&mut s, "s <- 'a'; s[3] <- 'c'; s", strs(&["a", "NA", "c"]));
        check(&mut s, "q <- NULL; q[3] <- 1; q", reals(&[na_real(), na_real(), 1.0]));
        check(&mut s, "r <- NULL; r$a <- 1; r", Val::List(vec![real(1.0)]));
        check(&mut s, "t2 <- NULL; t2[['k']] <- 5; t2", Val::List(vec![real(5.0)]));
        check(&mut s, "lst <- list(1, 2, 3); lst[[2]] <- NULL; lst", Val::List(vec![real(1.0), real(3.0)]));
        check(&mut s, "lst[2] <- list(NULL); lst", Val::List(vec![real(1.0), Val::Null]));
        check(&mut s, "a5 <- c(a = 1, b = 2); a5[['c']] <- 3; names(a5)", strs(&["a", "b", "c"]));
        check(&mut s, "a5[c('d', 'a')] <- c(10, 20); a5", reals(&[20.0, 2.0, 3.0, 10.0]));
        check(&mut s, "names(a5)", strs(&["a", "b", "c", "d"]));
    }
}

#[test]
fn complex_assignment() {
    for mut s in sessions() {
        check(&mut s, "n <- c(1, 2, 3); names(n) <- c('x', 'y', 'z'); names(n)", strs(&["x", "y", "z"]));
        check(&mut s, "names(n)[2] <- 'B'; names(n)", strs(&["x", "B", "z"]));
        check(&mut s, "n", reals(&[1.0, 2.0, 3.0]));
        check(
            &mut s,
            "m <- list(a = list(b = list(c = 1))); m$a$b$c <- 2; m[['a']][['b']][['d']] <- 3; m$a$b",
            Val::List(vec![real(2.0), real(3.0)]),
        );
        check(&mut s, "names(m$a$b)", strs(&["c", "d"]));
        // a closure's argument is copied before it is written
        check(&mut s, "f <- function(x) { x[1] <- 100; x }; o <- c(1, 2); f(o)", reals(&[100.0, 2.0]));
        check(&mut s, "o", reals(&[1.0, 2.0]));
        check(&mut s, "g <- function() { k <- 1:3; k[[2]] <- 5L; k }; g()", ints(&[1, 5, 3]));
        check(&mut s, "h <- function(v) { v$a <- 2; v }; ll <- list(a = 1); h(ll)", Val::List(vec![real(2.0)]));
        check(&mut s, "ll", Val::List(vec![real(1.0)]));
        // a user's replacement function, through `*tmp*`
        check(
            &mut s,
            "`second<-` <- function(x, value) { x[2] <- value; x }; u <- 1:3; second(u) <- 9L; u",
            ints(&[1, 9, 3]),
        );
        // the value of an assignment is its right-hand side, invisible
        let (v, vis) = s.run("p <- c(1, 2); p[2] <- 5").unwrap();
        assert!(same(&v, &real(5.0)) && !vis);
        // `*tmp*` holds the target while the index is evaluated, and is
        // gone after
        check(&mut s, "gg <- function(x) { x[{ record(`*tmp*`); 2 }] <- 10L; x }; gg(1:3)", ints(&[1, 10, 3]));
        assert_eq!(log_take(), vec!["Int([1, 2, 3])"]);
        assert_eq!(s.err("`*tmp*`"), "object '*tmp*' not found");
        // the right-hand side's promise keeps its expression while a getter
        // runs (a replacement function sees it through substitute)
        s.val("g <- function(x) x; `g<-` <- function(x, value) value; `f<-` <- function(x, value) substitute(value)");
        check(&mut s, "y <- 1; f(g(y)) <- 2 + 3; length(y)", int(3));
        assert_eq!(s.val("y[[1]]"), Val::Other(1));
        check(&mut s, "y[[3]]", real(3.0));
        // `$<-` NULL on a pairlist's first element: the rest is the result
        check(&mut s, "p <- pairlist(a = 1, b = 2, c = 3); p$a <- NULL; names(p)", strs(&["b", "c"]));
        check(&mut s, "p$b", real(2.0));
        check(&mut s, "p$c <- NULL; names(p)", str1("b"));
        check(&mut s, "length(p)", int(1));
        // a value pending in a builtin's arguments is not written in place
        check(&mut s, "x <- c(1, 2); list(x, { x[1] <- 5; x })[[1]]", reals(&[1.0, 2.0]));
    }
}

#[test]
fn subassignment_errors_report_gnu_rs_calls() {
    for mut s in sessions() {
        s.val("x <- 1:3");
        assert_eq!(s.err("x[[5]] <- 1:2"), "[<-] more elements supplied than there are to replace");
        assert_eq!(
            s.err("x[2] <- quote(a)"),
            "[<-] incompatible types (from integer to symbol) in subassignment type fix"
        );
        assert_eq!(s.err("x[[c(1, 2)]] <- 1"), "[[[<-] attempt to select more than one element in vectorIndex");
        assert_eq!(s.err("x[[0]] <- 1L"), "[<-] attempt to select less than one element in OneIndex <real>");
        assert_eq!(s.err("x[c(NA, 1)] <- 1:2"), "[<-] NAs are not allowed in subscripted assignments");
        assert_eq!(s.err("undefined_thing[1] <- 2"), "object 'undefined_thing' not found");
        assert_eq!(s.err("x[c(-1, 2)]"), "[[] only 0's may be mixed with negative subscripts");
        assert_eq!(s.err("(1:3)[[4]]"), "[[[] subscript out of bounds");
        assert_eq!(s.err("(1:3)$a"), "[$] $ operator is invalid for atomic vectors");
        assert_eq!(s.err("e <- new.env(); e[[1]]"), "[[[] wrong arguments for subsetting an environment");
    }
}

#[test]
fn switch_selects_and_falls_through() {
    for mut s in sessions() {
        check(&mut s, "switch('b', a = 1, b = 2, 3)", reals(&[2.0]));
        check(&mut s, "switch('z', a = 1, b = 2, 3)", reals(&[3.0]));
        check(&mut s, "switch(2, 'x', 'y', 'z')", str1("y"));
        check(&mut s, "switch('a', a = , b = 'ab', 'd')", str1("ab"));
        let (v, vis) = s.run("switch('q', a = 1)").unwrap();
        assert!(v == Val::Null && !vis);
        // a `next` and a `return` in an alternative
        check(&mut s, "k <- 0; for (i in 1:4) switch(i, next, k <- k + 1, next, k <- k + 10); k", reals(&[11.0]));
        check(&mut s, "fr <- function(x) { switch(x, a = return('early')); 'late' }; fr('a')", str1("early"));
    }
}

#[test]
fn substitute_and_on_exit() {
    for mut s in sessions() {
        s.val("sf <- function(x, y) substitute(x + y)");
        let v = s.val("deparsed <- sf(a * 2, b); deparsed[[1]]");
        assert_eq!(v, Val::Other(1)); // the symbol `+`
        check(&mut s, "length(deparsed)", int(3));
        check(&mut s, "length(deparsed[[2]])", int(3));
        check(&mut s, "sd <- function(x = 3 * 4) substitute(x); length(sd())", int(3));
        s.val("oe <- function() { on.exit(record('bye')); record('hi'); 1 }");
        check(&mut s, "oe()", real(1.0));
        assert_eq!(log_take(), vec!["hi", "bye"]);
        s.val(
            "oe2 <- function() { on.exit(record('1')); on.exit(record('2'), add = TRUE); \
             on.exit(record('0'), add = TRUE, after = FALSE); NULL }",
        );
        s.val("oe2()");
        assert_eq!(log_take(), vec!["0", "1", "2"]);
    }
}

/// `EnlargeVector`'s growth (docs/g3.md §4): an unshared vector grows in
/// place while it has room, else it is reallocated 5% larger, so a vector
/// grown by one element at a time moves exactly as often as GNU R's. The
/// counts are GNU R 4.6.2's (`.Internal(inspect(x))`'s address after each
/// step): 59 moves for 200 list elements, 91 for 1000 doubles.
///
/// The test reads addresses, so under `ROBIGO_GC_MOVE=1` (every collection
/// moves objects) it skips and says so in the log: the other passes count
/// it.
#[test]
fn growth_reallocates_as_gnu_r_does() {
    if std::env::var("ROBIGO_GC_MOVE").as_deref() == Ok("1") {
        // to the stream itself: libtest captures `eprintln!` of a passing test
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "growth_reallocates_as_gnu_r_does: skipped, under ROBIGO_GC_MOVE=1 an address does not identify an object"
        );
        return;
    }
    for mut s in sessions() {
        check(
            &mut s,
            "l <- list(); n <- 0; prev <- 0; \
             for (i in 1:200) { l[[i]] <- i; a <- addr(l); if (a != prev) n <- n + 1; prev <- a }; n",
            real(59.0),
        );
        check(&mut s, "length(l)", int(200));
        check(
            &mut s,
            "v <- c(1)[0]; n <- 0; prev <- 0; \
             for (i in 1:1000) { v[i] <- i; a <- addr(v); if (a != prev) n <- n + 1; prev <- a }; n",
            real(91.0),
        );
        // the same inside a closure, where the target is a local
        check(
            &mut s,
            "f <- function() { v <- c(1)[0]; n <- 0; prev <- 0; \
             for (i in 1:1000) { v[i] <- i; a <- addr(v); if (a != prev) n <- n + 1; prev <- a }; n }; f()",
            real(91.0),
        );
        // and with names, which grow alike
        check(
            &mut s,
            "w <- c(a = 1); for (i in 1:30) w[[.Internal(paste0(list('k', i), NULL, FALSE))]] <- i; length(w)",
            int(31),
        );
        check(&mut s, "names(w)[c(1, 2, 31)]", strs(&["a", "k1", "k30"]));
        check(&mut s, "w[['k30']]", reals(&[30.0]));
    }
}

/// A link raised on a value while later arguments are evaluated is dropped
/// when one of them fails, not left on the value for good (GNU R's jumps
/// reset the stack's protection as they pass it): `evalList`'s values,
/// `R_DispatchOrEvalSP`'s object, `applydefine`'s right-hand side. A leaked
/// link makes the value look shared, so a later assignment copies it.
#[test]
fn links_are_dropped_on_an_early_exit() {
    for mut s in sessions() {
        s.val("x <- c(1, 2); y <- c(3, 4); r0 <- refcnt(x); q0 <- refcnt(y)");
        // evalList: `...` makes the builtin's arguments evaluate at run time
        s.val("f <- function(...) count(x, ..., nope)");
        assert_eq!(s.err("f()"), "[f] object 'nope' not found");
        check(&mut s, "refcnt(x) - r0", int(0));
        // `[`'s object while its index fails
        assert_eq!(s.err("x[nope]"), "object 'nope' not found");
        check(&mut s, "refcnt(x) - r0", int(0));
        // complex assignment's right-hand side while the setter fails
        assert_eq!(s.err("z <- list(); z[[nope]] <- y"), "object 'nope' not found");
        check(&mut s, "refcnt(y) - q0", int(0));
    }
}
