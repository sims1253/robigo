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
