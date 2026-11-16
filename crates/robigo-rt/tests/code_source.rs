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

/// Marking walks a worklist, not the native stack: a call nested 200,000
/// deep (data C can build and quote) is marked when a write puts it under a
/// marked `quote()`, on a test thread's 2 MiB stack. Allocating it under
/// gctorture or the moving collector would take a collection per cell, so
/// the test skips there.
#[test]
fn deeply_nested_calls_are_marked_without_recursion() {
    if std::env::var("ROBIGO_GCTORTURE").as_deref() == Ok("1") {
        use std::io::Write;
        let _ =
            writeln!(std::io::stderr(), "deeply_nested_calls_are_marked_without_recursion: skipped under gctorture");
        return;
    }
    use robigo_rtcore::{CodeSource, RSym};
    let mut s = Session::new(Profile::Eval);
    let cx = s.cx();
    cx.scope(|cx, sc| -> robigo_rtcore::RResult<()> {
        // quote(x), marked as compiled code's
        let x = sc.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"x")?.into())))?;
        let call = sc.capture(cx, |cx, o| cx.build(o, |cx| cx.lang2(cx.sym(RSym::Quote).into(), x.bind(cx))))?;
        cx.mark_code_source(call.bind(cx), CodeSource::Ast);
        // g(g(...g(1)...))
        let g = sc.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"g")?.into())))?;
        let e = sc.capture(cx, |cx, o| cx.build_scalar(o, 1i32))?;
        for _ in 0..200_000 {
            e.replace_with(cx, |cx| cx.lang2(g.bind(cx), e.bind(cx)))?;
        }
        let arg = sc.root(cx, call.bind(cx).as_cons().unwrap().cdr());
        cx.set_car(arg, e);
        Ok(())
    })
    .expect("no R error");
    // the write into the marked cell made AST code stale
    assert!(s.cx().code_floors().0 > 0);
}
