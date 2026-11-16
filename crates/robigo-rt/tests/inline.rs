//! Pure-closure inlining (docs/g3.md §9.2, §12): calls from compiled code
//! that run inline keep everything the real calls make that R can see. The
//! sessions take GNU R's JIT decisions (no forced profile, which turns the
//! inliner off), so the closures are compiled after their first calls, and
//! each program runs again with `ROBIGO_NO_INLINE`'s switch off: values,
//! errors, `R_Visible`, `eval`'s and `bcEval`'s counts, and the depth limit
//! must not differ.

mod common;

use common::*;
use robigo_rtcore::{Cx, Ref};

/// What a run leaves that R can observe: the value or the error, the
/// visibility, and the evaluation counts it made from zero.
#[derive(Debug, PartialEq)]
struct Seen {
    result: Result<(Val, bool), String>,
    evalcount: i32,
    bc_evalcount: i32,
}

fn seen(r: &mut Session, src: &str) -> Seen {
    let cx = r.cx();
    cx.evalcount().set(0);
    cx.bc_evalcount().set(0);
    let result = r.run(src);
    let cx = r.cx();
    Seen { result, evalcount: cx.evalcount().get(), bc_evalcount: cx.bc_evalcount().get() }
}

/// `src` run with inlining on and then off: what each leaves, and the calls
/// run inline. Two runs first settle the JIT's decisions (a closure is
/// compiled at its second call), which would differ between the two
/// otherwise.
fn both(r: &mut Session, src: &str) -> (Seen, Seen, u64) {
    let _ = (r.run(src), r.run(src));
    let before = robigo_rt::inline_runs(r.cx());
    let on = seen(r, src);
    let runs = robigo_rt::inline_runs(r.cx()) - before;
    robigo_rt::set_inline(r.cx(), false);
    let off = seen(r, src);
    robigo_rt::set_inline(r.cx(), true);
    (on, off, runs)
}

/// The global binding of `name`, a closure.
fn closure(cx: &mut Cx, name: &str) -> robigo_heap::Sexp {
    cx.scope(|cx, s| {
        let sym = cx.install_rooted(s, name.as_bytes()).unwrap();
        let g = s.root(cx, cx.global_env());
        let out = s.out(cx);
        let f = out.root();
        cx.find_var(sym, g, out).unwrap();
        let x = f.bind(cx).sexp();
        // SAFETY: a live closure, bound in the global environment.
        assert_eq!(unsafe { x.typ() }, robigo_heap::types::CLOSXP);
        x
    })
}

const PROGRAMS: &str = r#"
fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
tag <- function(x, label) if (label == "bc") x * 2 else x
sb <- function(x, scale = 1, label = "a") if (label == "zz") x else x * scale
pick <- function(x, y, z) if (x) y else z
lg <- function(a, b) c(a && b, a || b)
k <- function() 42
loop <- function(n) { s <- 0; for (i in 1:n) s <- s + tag(i * 0.25, "bc") + sb(i) + pick(i > 2, 1, 2); s }
weigh <- function(x, y) { z <- x * y; z + 1 }
w <- 100
ul <- function(x) { if (x > 5) w <- 1; w + x }
sw <- function(x) { x <- x * 2; if (x > 4) x else "small" }
locs <- function(n) { s <- 0; for (i in 1:n) s <- s + weigh(y = 2, x = i) + ul(i); s }
"#;

#[test]
fn inlined_calls_leave_what_real_calls_leave() {
    let mut r = Session::jit();
    r.val(PROGRAMS);
    // the first calls compile the closures
    for _ in 0..3 {
        r.val("loop(10); fib(8); locs(3)");
    }
    let mut runs = 0;
    for src in [
        "fib(12)",
        "loop(30)",
        "c(k(), pick(1L, 3, 4), pick(0, 3, 4))",
        "lg(TRUE, NA)",
        "for (i in 1:5) z <- fib(i); z",
        "tag(1, NA_character_)",
        "tag(\"a\", \"bc\")",
        "sb(2, label = \"zz\")",
        "locs(12)",
        "c(sw(1), sw(5))",
        "weigh(\"a\", 2)",
    ] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        runs += n;
    }
    assert!(runs > 50, "inline runs: {runs}");
}

#[test]
fn debugged_closures_are_not_inlined() {
    let mut r = Session::jit();
    r.val("f <- function(x) x + 1; g <- function() { s <- 0; for (i in 1:20) s <- s + f(i); s }");
    for _ in 0..3 {
        r.val("g()");
    }
    let n0 = robigo_rt::inline_runs(r.cx());
    assert_eq!(r.val("g()"), Val::Real(vec![230.0]));
    assert!(robigo_rt::inline_runs(r.cx()) > n0, "the warm loop runs f inline");
    // GNU R's debug, trace and step bits (and NOJIT): bits 0-3 of `extra`
    for bit in 0..4u16 {
        let f = closure(r.cx(), "f");
        // SAFETY: the closure is live; only robigo's own bits change.
        unsafe { f.set_extra(f.extra() | 1 << bit) };
        let n = robigo_rt::inline_runs(r.cx());
        assert_eq!(r.val("g()"), Val::Real(vec![230.0]));
        assert_eq!(robigo_rt::inline_runs(r.cx()), n, "f inlined with bit {bit}");
        let f = closure(r.cx(), "f");
        // SAFETY: as above.
        unsafe { f.set_extra(f.extra() & !(1 << bit)) };
    }
}

#[test]
fn the_depth_limit_falls_where_real_calls_put_it() {
    let mut r = Session::jit();
    r.val("d <- function(n) if (n == 0) 0 else d(n - 1) + 1; h <- function(n) d(n)");
    for _ in 0..3 {
        r.val("h(5)");
    }
    let mut runs = 0;
    for limit in [40, 77] {
        r.cx().set_expressions(limit);
        let mut errs = 0;
        for n in 0..limit {
            let src = format!("h({n})");
            let (on, off, k) = both(&mut r, &src);
            assert_eq!(on, off, "{src} with expressions = {limit}");
            runs += k;
            errs += on.result.is_err() as u32;
        }
        assert!(errs > 0, "no depth error under expressions = {limit}");
    }
    r.cx().set_expressions(5000);
    assert!(runs > 0, "no inline run");
}

/// Formals written in place (package C's `SETCDR`) after the closure ran
/// inline: its code is copied for the new formals at the next call
/// (`code::reshape`), and the copy is judged again. Here the new formals
/// are more than an inline frame holds, so the calls are real ones.
#[test]
fn code_reshaped_for_new_formals_is_judged_again() {
    let mut r = Session::jit();
    r.val(
        "f <- function(x) x + 1; g <- function() { s <- 0; for (i in 1:20) s <- s + f(i); s }; \
         h <- function(a = 1, b = 2, c = 3, d = 4) NULL",
    );
    for _ in 0..3 {
        r.val("g()");
    }
    let n0 = robigo_rt::inline_runs(r.cx());
    assert_eq!(r.val("g()"), Val::Real(vec![230.0]));
    assert!(robigo_rt::inline_runs(r.cx()) > n0, "the warm loop runs f inline");
    let f = closure(r.cx(), "f");
    // (h's formals, as the new tail)
    let tl = closure(r.cx(), "h");
    r.cx().scope(|cx, s| {
        // SAFETY: f's and h's formals are live objects of this heap,
        // reachable from the global environment.
        let (cell, tail) =
            unsafe { (s.root(cx, Ref::from_sexp(f.car(), cx)), s.root(cx, Ref::from_sexp(tl.car(), cx))) };
        cx.set_cdr(cell, tail);
    });
    let (on, off, runs) = both(&mut r, "g()");
    assert_eq!(on, off);
    assert_eq!(on.result.map(|(v, _)| v), Ok(Val::Real(vec![230.0])));
    assert_eq!(runs, 0, "f ran inline with five formals");
}

/// A body that assigns a local runs inline; one that reassigns a formal
/// does not: the real call's promise for the formal, dropped from the
/// frame by the store, keeps references that an inline run would not make
/// (tests/rt/capi/inline-reshape.R shows them).
#[test]
fn bodies_that_reassign_a_formal_are_not_inlined() {
    let mut r = Session::jit();
    r.val(PROGRAMS);
    for _ in 0..3 {
        r.val("weigh(1, 2); sw(1); sw(3)");
    }
    for (src, inline) in [("weigh(3, 4)", true), ("sw(1)", false), ("sw(3)", false)] {
        let (on, off, runs) = both(&mut r, &format!("for (i in 1:3) v <- {src}; v"));
        assert_eq!(on, off, "{src}");
        assert_eq!(runs > 0, inline, "{src}: {runs} inline runs");
    }
}
