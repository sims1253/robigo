//! GNU R's JIT decisions (`crate::jit`, docs/g3.md §6.1, §9.1) driven
//! from R code in a session whose profiles are not forced: the closure
//! states `R_CheckJIT` sets, the JIT cache, closures and promises compiled
//! code makes, top-level loops, `environment<-`. The R-level cases
//! recorded on GNU R 4.6.2 are `tests/rt/jit/`; these run in-process, so
//! `scripts/rt-ci.py` runs them under gctorture and the moving collector.

mod common;

use common::*;

fn s(x: &str) -> Val {
    str1(x)
}

#[test]
fn r_check_jit_marks_closures_as_gnu_r_does() {
    let mut r = Session::jit();
    // a small top-level function: MAYBEJIT at its first call, compiled at
    // its second
    r.val("small <- function() 1");
    assert_eq!(r.val("jitstate(small)"), s("unseen"));
    r.val("small()");
    assert_eq!(r.val("jitstate(small)"), s("maybe"));
    r.val("small()");
    assert_eq!(r.val("jitstate(small)"), s("compiled"));
    // a loop scores LOOP_JIT_SCORE: compiled at the first call
    r.val("big <- function() { for (i in 1:2) NULL; 1 }");
    r.val("big()");
    assert_eq!(r.val("jitstate(big)"), s("compiled"));
    // small and not top-level: NOJIT at once
    r.val("mk <- function() { if (FALSE) browser(); function() 1 }");
    r.val("inner <- mk()");
    r.val("inner()");
    assert_eq!(r.val("jitstate(inner)"), s("nojit"));
    // mk may call browser(): compiled never
    r.val("mk()");
    r.val("mk()");
    assert_eq!(r.val("jitstate(mk)"), s("nojit"));
    // big and not top-level: MAYBEJIT, then compiled
    r.val("mkbig <- function() { if (FALSE) browser(); function() { for (i in 1) NULL; 2 } }");
    r.val("b2 <- mkbig()");
    r.val("b2()");
    assert_eq!(r.val("jitstate(b2)"), s("maybe"));
    assert_eq!(r.val("b2()"), real(2.0));
    assert_eq!(r.val("jitstate(b2)"), s("compiled"));
}

#[test]
fn compiled_code_makes_compiled_closures_and_byte_code_promises() {
    let mut r = Session::jit();
    r.val("bt <- function(a) bindingtype(quote(a))");
    r.val("mk <- function() function() bt(1)");
    r.val("f1 <- mk()");
    assert_eq!(r.val("f1()"), s("delayed"));
    assert_eq!(r.val("jitstate(f1)"), s("nojit"));
    r.val("f2 <- mk()");
    assert_eq!(r.val("jitstate(mk)"), s("compiled"));
    assert_eq!(r.val("jitstate(f2)"), s("compiled"));
    // compiled code's constant argument is a value; a call's is a promise
    // whose code is byte code
    assert_eq!(r.val("f2()"), s("value"));
    r.val("g <- function(x) { for (i in 1) NULL; h(x + 1) }");
    r.val("h <- function(y) list(bindingtype(quote(y)), y, bindingtype(quote(y)))");
    assert_eq!(r.val("g(1)"), Val::List(vec![s("delayed"), real(2.0), s("forced")]));
    assert_eq!(r.val("jitstate(g)"), s("compiled"));
}

#[test]
fn top_level_loops_are_compiled_as_they_start() {
    let mut r = Session::jit();
    r.val("bt <- function(a) bindingtype(quote(a))");
    assert_eq!(r.val("bt(1)"), s("delayed"));
    r.val("out <- NULL");
    r.val("for (i in 1:3) out <- bt(1)");
    assert_eq!(r.val("out"), s("value"));
    r.val("while (TRUE) { out <- bt(1); break }");
    assert_eq!(r.val("out"), s("value"));
    r.val("repeat { out <- bt(1); break }");
    assert_eq!(r.val("out"), s("value"));
    // a closure a compiled top-level loop makes is compiled
    r.val("for (i in 1) mkd <- function() bt(1)");
    assert_eq!(r.val("jitstate(mkd)"), s("compiled"));
    assert_eq!(r.val("mkd()"), s("value"));
    // a loop inside an interpreted function's frame is not
    r.val("loc <- function() { if (FALSE) browser(); for (i in 1) out <- bt(1); out }");
    assert_eq!(r.val("loc()"), s("delayed"));
}

#[test]
fn the_jit_cache_shares_code_and_refuses_other_environments() {
    let mut r = Session::jit();
    r.val("bt <- function(a) bindingtype(quote(a))");
    // compiled at its first call (a loop), entered in the cache
    r.val("t1 <- function() { for (i in 1) NULL; bt(1) }");
    assert_eq!(r.val("t1()"), s("value"));
    // an identical body in a frame binding a name the entry lacks: NOJIT
    // at the call that would compile it
    r.val("mk <- function() { z <- 1; if (FALSE) browser(); function() { for (i in 1) NULL; bt(1) } }");
    r.val("t2 <- mk()");
    assert_eq!(r.val("t2()"), s("delayed"));
    assert_eq!(r.val("jitstate(t2)"), s("maybe"));
    assert_eq!(r.val("t2()"), s("delayed"));
    assert_eq!(r.val("jitstate(t2)"), s("nojit"));
    // the same body and environment, other formals objects: the cached
    // code, with the decisions it was compiled with (`+` was base's)
    r.val("f <- function(x) x + 1");
    r.val("f(1)");
    r.val("f(1)");
    r.val("`+` <- function(e1, e2) 99");
    r.val("g <- function(x) x + 1");
    assert_eq!(r.val("g(1)"), real(99.0));
    assert_eq!(r.val("g(1)"), real(2.0));
    assert_eq!(r.val("jitstate(g)"), s("compiled"));
}

#[test]
fn environment_assignment_returns_a_compiled_closure_to_its_source() {
    let mut r = Session::jit();
    r.val("bt <- function(a) bindingtype(quote(a))");
    r.val("e <- function() bt(1)");
    assert_eq!(r.val("e()"), s("delayed"));
    assert_eq!(r.val("e()"), s("value"));
    assert_eq!(r.val("jitstate(e)"), s("compiled"));
    r.val("environment(e) <- globalenv()");
    // MAYBEJIT was cleared when it compiled: the JIT decides again
    assert_eq!(r.val("jitstate(e)"), s("unseen"));
    assert_eq!(r.val("e()"), s("delayed"));
    assert_eq!(r.val("e()"), s("value"));
    // the bits stay: NOJIT is kept
    r.val("mk <- function() { if (FALSE) browser(); function() bt(1) }");
    r.val("n <- mk()");
    r.val("n()");
    assert_eq!(r.val("jitstate(n)"), s("nojit"));
    r.val("environment(n) <- globalenv()");
    assert_eq!(r.val("jitstate(n)"), s("nojit"));
    // a copy shares the compiled body (duplicate)
    r.val("c1 <- function() bt(1)");
    r.val("c1(); c1()");
    r.val("c2 <- c1");
    r.val("environment(c2) <- globalenv()");
    assert_eq!(r.val("jitstate(c1)"), s("compiled"));
    assert_eq!(r.val("jitstate(c2)"), s("unseen"));
    assert_eq!(r.err("environment(c1) <- NULL"), "[<-] use of NULL environment is defunct");
    assert_eq!(r.err("environment(c1) <- 1"), "[<-] replacement object is not an environment");
}

#[test]
fn a_callback_from_a_compiled_internal_keeps_the_expression_it_is_at() {
    // the internal's expression (the `.Internal` call) is saved across the
    // activation its callback runs, and rooted there: rt-ci runs this
    // under gctorture with the moving collector
    let mut r = Session::jit();
    r.val("g <- function() { x <- 1; x + 1 }");
    r.val("f <- function() { for (i in 1) NULL; .Internal(do.call(g, list(), globalenv())) }");
    assert_eq!(r.val("f()"), real(2.0));
    assert_eq!(r.val("jitstate(f)"), s("compiled"));
    for _ in 0..3 {
        assert_eq!(r.val("f()"), real(2.0));
    }
    // and an error() after it reports the function's call, as the AST
    // would for a closure wrapper's .Internal
    r.val("h <- function() { for (i in 1) NULL; .Internal(do.call(g, 1, globalenv())) }");
    let m = r.err("h()");
    assert!(m.starts_with("[h]"), "{m}");
}

#[test]
fn constant_folds_share_the_pools_identical_constants() {
    // cmp.R's constant folding: a fold's value joins the identical()
    // constant already in the pool (0 and -0 are one), folds nest (one
    // compile round each), a folded test drops the branch it does not take,
    // and a fold that errs is left to run. The rounds evaluate R code
    // between tries of one compile: rt-ci runs this under gctorture with
    // the moving collector.
    let mut r = Session::jit();
    let inf = real(f64::INFINITY);
    let ninf = real(f64::NEG_INFINITY);
    r.val("zero <- function() { a <- 0; b <- -0; for (i in 1) v <- b; 1 / v }");
    assert_eq!(r.val("zero()"), inf);
    assert_eq!(r.val("jitstate(zero)"), s("compiled"));
    r.val("alone <- function() { b <- -0; for (i in 1) v <- b; 1 / v }");
    assert_eq!(r.val("alone()"), ninf);
    r.val("nested <- function() { a <- 0; b <- -(-(-(1 - 1))); for (i in 1) v <- b; 1 / v }");
    assert_eq!(r.val("nested()"), inf);
    r.val("dead <- function() { if (2 < 1) a <- 0; b <- -0; for (i in 1) v <- b; 1 / v }");
    assert_eq!(r.val("dead()"), ninf);
    r.val("live <- function() { if (1 < 2) a <- 0 else a <- 1; b <- -0; for (i in 1) v <- b; 1 / v }");
    assert_eq!(r.val("live()"), inf);
    // the fold of 1 + "a" errs when the code is compiled: not folded, the
    // error raised where the code runs
    r.val("errs <- function(n) { for (i in 1) if (n > 5) 1 + \"a\"; n }");
    assert_eq!(r.val("errs(1)"), real(1.0));
    assert_eq!(r.val("jitstate(errs)"), s("compiled"));
    assert_eq!(r.err("errs(9)"), "[+] non-numeric argument to binary operator");
    // a top-level loop's code folds too
    r.val("for (i in 1) { a <- 0; b <- -0 }");
    assert_eq!(r.val("1 / b"), inf);
}

#[test]
fn switch_compiles_as_cmp_r_does() {
    let mut r = Session::jit();
    r.val("f <- function(x) { for (i in 1) NULL; switch(x, a = \"A\", b = , c = \"BC\", \"other\") }");
    for (x, want) in [("\"a\"", "A"), ("\"b\"", "BC"), ("\"c\"", "BC"), ("\"d\"", "other"), ("NA_character_", "other")]
    {
        assert_eq!(r.val(&format!("f({x})")), s(want), "{x}");
    }
    assert_eq!(r.val("jitstate(f)"), s("compiled"));
    // NA matches the name "NA"; EXPR's tag is ignored
    r.val("k <- function(x) { for (i in 1) NULL; switch(x, \"NA\" = \"na\", \"x\") }");
    assert_eq!(r.val("k(NA_character_)"), s("na"));
    r.val("t <- function() { for (i in 1) NULL; switch(a = \"b\", EXPR = \"a\", b = \"B\") }");
    assert_eq!(r.val("t()"), s("B"));
    r.val("h <- function(n) { for (i in 1) NULL; switch(n, \"one\", \"two\", \"three\") }");
    assert_eq!(r.val("h(2)"), s("two"));
    assert_eq!(r.val("h(2.9)"), s("two"));
    assert_eq!(r.val("h(7)"), Val::Null);
    assert_eq!(r.err("h(c(1, 2))"), "[switch] EXPR must be a length 1 vector");
    assert_eq!(r.err("h(\"a\")"), "[switch] numeric EXPR required for 'switch' without named alternatives");
}

#[test]
fn compiled_subsetting_dispatches_before_its_indices() {
    let mut r = Session::jit();
    r.val("x <- c(10, 20, 30)");
    r.val("f <- function(v, i) { for (k in 1) NULL; v[i] }");
    assert_eq!(r.val("f(x, 2)"), real(20.0));
    assert_eq!(r.val("jitstate(f)"), s("compiled"));
    r.val("g <- function(v, i) { for (k in 1) NULL; v[[i]] }");
    assert_eq!(r.val("g(x, 3)"), real(30.0));
    assert_eq!(r.err("g(x, 4)"), "[[[] subscript out of bounds");
    // an index bound to a missing argument is an empty index
    r.val("m <- function(i) { for (k in 1) NULL; x[i] }");
    assert_eq!(r.val("m()"), Val::Real(vec![10.0, 20.0, 30.0]));
    r.val("m2 <- function(j) m(j)");
    assert_eq!(r.val("m2()"), Val::Real(vec![10.0, 20.0, 30.0]));
    // `$` on a list, and on an object whose method sees the call's promises
    r.val("l <- list(1, 2)");
    r.val("names(l) <- c(\"p\", \"q\")");
    r.val("d <- function(v) { for (k in 1) NULL; v$q }");
    assert_eq!(r.val("d(l)"), real(2.0));
    r.val("`[.foo` <- function(x, i) list(record(\"method\"), i)");
    r.val("o <- 1");
    r.val("class(o) <- \"foo\"");
    r.val("h <- function(o, j) { for (k in 1) NULL; o[j + 1] }");
    assert_eq!(r.val("h(o, 1)"), Val::List(vec![Val::Null, real(2.0)]));
    assert_eq!(log_take(), vec!["method"]);
}

#[test]
fn compiled_complex_assignment_binds_no_tmp() {
    let mut r = Session::jit();
    r.val("f <- function() { x <- c(1, 2, 3); for (i in 1:2) x[i] <- i * 10; x }");
    assert_eq!(r.val("f()"), Val::Real(vec![10.0, 20.0, 3.0]));
    assert_eq!(r.val("jitstate(f)"), s("compiled"));
    r.val("g <- function() { l <- list(1, 2); for (i in 1) { names(l) <- c(\"a\", \"b\"); l$b <- 5; l[[\"a\"]] <- 4 }; l }");
    assert_eq!(r.val("g()"), Val::List(vec![real(4.0), real(5.0)]));
    r.val("y <- c(1, 2)");
    r.val("h <- function() { for (i in 1) y[2] <<- 7; y }");
    assert_eq!(r.val("h()"), Val::Real(vec![1.0, 7.0]));
    assert_eq!(r.val("y"), Val::Real(vec![1.0, 7.0]));
}
