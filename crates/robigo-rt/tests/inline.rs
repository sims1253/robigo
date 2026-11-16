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

/// Closures GNU R's JIT never compiles (small, defined in a function: their
/// `Eval` code, GNU R's AST) that write their enclosure with `<<-` and
/// return `invisible(...)`. Their calls from compiled code run inline,
/// with each `<<-` buffered and committed at the end. A run that gives up
/// after a write, at an overflow warning or an error, drops it, and the
/// real call writes once.
const SUPER: &str = r#"
acct <- function() { bal <- 0; list(dep = function(x) { bal <<- bal + x; invisible(bal) }, get = function() bal) }
pair <- function() { n <- 0L; s <- 0; list(add = function(x) { n <<- n + 1L; s <<- s + x; invisible(s) }, get = function() c(n, s)) }
ov <- function() { bal <- 0L; list(dep = function(x) { bal <<- bal + 1L; invisible(x + 2147483647L) }, get = function() bal) }
bad <- function() { bal <- 0; list(dep = function(x) { bal <<- bal + 1; invisible(x + "a") }, get = function() bal) }
fresh <- function() function(x) { newglob <<- x; invisible(x) }
glob <- function() function(x) { tot <<- tot + x; invisible(tot) }
r1 <- function() { a <- acct(); for (i in 1:60) a$dep(i %% 7); a$get() }
r2 <- function() { a <- acct(); f <- function() a$dep(1); for (i in 1:5) v <- f(); c(v, a$get()) }
r3 <- function() { p <- pair(); for (i in 1:30) p$add(i); p$get() }
r4 <- function() { tot <<- 0; h <- glob(); for (i in 1:20) h(i); tot }
r5 <- function() { o <- ov(); for (i in 1:5) w <- tryCatch(o$dep(1L), warning = function(w) conditionMessage(w)); c(w, o$get()) }
r6 <- function() { b <- bad(); for (i in 1:5) e <- tryCatch(b$dep(1), error = function(e) conditionMessage(e)); c(e, b$get()) }
r7 <- function() { g <- fresh(); for (i in 1:5) g(i); newglob }
vis <- function() { a <- acct(); for (i in 1:3) a$dep(1); a$dep(1) }
"#;

/// Whether every allocation collects (rt-ci's gctorture step): a `<<-`
/// value's box is then refused in every inline run, which gives up.
fn gctorture() -> bool {
    std::env::var("ROBIGO_GCTORTURE").as_deref() == Ok("1")
}

#[test]
fn superassigning_eval_bodies_leave_what_real_calls_leave() {
    let mut r = Session::jit();
    r.val(SUPER);
    let mut runs = 0;
    for (src, inline) in [
        ("r1()", true),
        ("r2()", true),
        ("r3()", true),
        ("r4()", true),
        ("vis()", true),
        ("r5()", false),
        ("r6()", false),
        // (the first `<<-` defines the global, in a real call; later ones
        // write it inline)
        ("r7()", true),
    ] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        if !gctorture() {
            assert_eq!(n > 0, inline, "{src}: {n} inline runs");
        }
        runs += n;
    }
    assert!(gctorture() || runs > 50, "inline runs: {runs}");
}

/// A `<<-` whose binding is locked or active is not buffered: the real call
/// raises the error or calls the binding's function.
#[test]
fn locked_and_active_targets_are_left_to_the_real_call() {
    let mut r = Session::jit();
    r.val(SUPER);
    r.val("a <- acct(); d <- a$dep; got <- NULL");
    r.val("aa <- (function() function(x) { ab <<- x; invisible(x) })()");
    r.val("l1 <- function() { for (i in 1:3) m <- tryCatch(a$dep(1), error = function(c) conditionMessage(c)); c(m, a$get()) }");
    r.val("l2 <- function() { got <<- NULL; for (i in 1:4) aa(i); got }");
    // the account's frame, `dep`'s environment: its `bal` locked
    let env = closure(r.cx(), "d");
    r.cx().scope(|cx, s| {
        // SAFETY: a live closure; its environment is a live frame.
        let e = s.root(cx, unsafe { Ref::from_sexp(env.tag(), cx) });
        let bal = cx.install_rooted(s, b"bal").unwrap();
        cx.lock_binding(bal, e).unwrap();
    });
    r.val("setter <- function(v) if (missing(v)) got else got <<- c(got, v)");
    r.cx().scope(|cx, s| {
        let g = s.root(cx, cx.global_env());
        let sym = cx.install_rooted(s, b"setter").unwrap();
        let o = s.out(cx);
        let f = o.root();
        cx.find_var(sym, g, o).unwrap();
        let ab = cx.install_rooted(s, b"ab").unwrap();
        cx.make_active_binding(ab, f, g).unwrap();
    });
    for src in ["l1()", "l2()"] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        assert_eq!(n, 0, "{src}: {n} inline runs");
    }
}

/// The depth limit falls at the same call when an inlined `Eval` body is
/// the innermost one.
#[test]
fn the_depth_limit_falls_where_real_calls_put_it_in_eval_bodies() {
    let mut r = Session::jit();
    r.val(SUPER);
    r.val("a <- acct(); deep <- function(n) if (n == 0) a$dep(1) else deep(n - 1) + 0; h <- function(n) deep(n)");
    for _ in 0..3 {
        r.val("h(3)");
    }
    let mut runs = 0;
    for limit in [40, 77] {
        r.cx().set_expressions(limit);
        let mut errs = 0;
        for n in 0..limit {
            let src = format!("h({n})");
            let (on, off, k) = both(&mut r, &src);
            assert_eq!(on.result.is_err(), off.result.is_err(), "{src} with expressions = {limit}");
            if on.result.is_err() {
                assert_eq!(on, off, "{src} with expressions = {limit}");
            }
            runs += k;
            errs += on.result.is_err() as u32;
        }
        assert!(errs > 0, "no depth error under expressions = {limit}");
    }
    r.cx().set_expressions(5000);
    assert!(gctorture() || runs > 0, "no inline run");
}

/// Fused steps (a formal compared with a scalar and branched on, a call of
/// a function looked up by name, a formal returned, arithmetic returned)
/// are shortcuts through the instructions they stand for: where their
/// preconditions fail (a string, NA, an integer overflow, an argument not
/// forced yet), the run takes the instructions one by one, or gives up
/// exactly where those would.
#[test]
fn fused_steps_leave_what_their_instructions_leave() {
    let mut r = Session::jit();
    r.val(
        "ci <- function(n) if (n < 2L) n else ci(n - 1L) + 1L; \
         ov <- function(n) if (n < 1L) n + 2147483647L else ov(n - 1L) + 0L; \
         fc <- function(n) if (n < 2) n else fc(n - 1) + fc(n - 2); \
         r1 <- function() c(ci(10L), fc(12), fc(0.5)); \
         r2 <- function() ci(1i); \
         r3 <- function() ci(NA_integer_); \
         r4 <- function() ov(3L); \
         r5 <- function() { s <- 0; for (i in 1:20) s <- s + fc(i %% 6); s }",
    );
    let mut runs = 0;
    for src in ["r1()", "r2()", "r3()", "r4()", "r5()"] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        runs += n;
    }
    assert!(runs > 20, "inline runs: {runs}");
}

/// Argument code of one of the shapes an inline run computes directly (a
/// variable, a variable and a scalar or a second variable under one
/// operator) gives what running it gives: integer overflow, NA, a string
/// operand and a non-number give up as the code would. (An NA result is
/// compared as `is.na`: NaN equals nothing.)
#[test]
fn argument_shapes_leave_what_their_code_leaves() {
    let mut r = Session::jit();
    r.val(
        "id <- function(x) x; add <- function(x) x + 1; \
         a1 <- function() { s <- 0; for (i in 1:20) s <- s + add(i) + add(2 * i) + add(i + i) + id(i - 1); s }; \
         a2 <- function() { s <- 0L; for (i in 1:5) s <- s + add(i + 2147483647L); is.na(s) }; \
         a3 <- function() { s <- 0; for (i in 1:5) s <- s + add(NA_real_ * i); is.na(s) }; \
         a4 <- function() { v <- \"a\"; add(v + 1) }; \
         a5 <- function() { s <- 0; for (i in 1:5) s <- s + add(i / 0) + add(-i %/% 2L); s }",
    );
    let mut runs = 0;
    for src in ["a1()", "a2()", "a3()", "a4()", "a5()"] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        runs += n;
    }
    assert!(runs > 50, "inline runs: {runs}");
}

/// Two unboxed doubles in an inline run's arithmetic and comparisons go
/// straight to the kernels' double arms. What that leaves is what the
/// general path leaves: which of NA and NaN survives, the sign of zero,
/// infinities, and the other operand types, through the fused steps and
/// argument shapes too. (NA and NaN are compared as 10 and 11: NaN equals
/// nothing.)
#[test]
fn double_operands_leave_what_the_general_path_leaves() {
    let mut r = Session::jit();
    r.val(
        "add <- function(x, y) x + y; sub <- function(x, y) x - y; mul <- function(x, y) x * y; \
         dvd <- function(x, y) x / y; pw <- function(x, y) x ^ y; md <- function(x, y) x %% y; \
         lt <- function(x, y) x < y; eq <- function(x, y) x == y; id <- function(x) x; \
         br <- function(n) if (n < 2) n else br(n - 1) + br(n - 2); \
         e <- function(x) if (is.na(x)) 10 + is.nan(x) else x; \
         ar <- function(a, b) { v <- NULL; for (k in 1:2) { p <- add(a, b); m <- sub(a, b); t <- mul(a, b); \
                                d <- dvd(a, b); w <- pw(a, b); o <- md(a, b); l <- lt(a, b); q <- eq(a, b); \
                                v <- list(e(p), e(m), e(t), e(d), e(w), e(o), e(l), e(q)) }; v }; \
         t1 <- function() list(ar(NA_real_, NaN), ar(NaN, NA_real_), ar(Inf, -Inf), ar(5L, 2), ar(TRUE, 2.5)); \
         t2 <- function() { v <- NULL; for (k in 1:2) { a <- br(10); b <- br(2.5); c <- br(-Inf); d <- br(1L); \
                            z <- sub(0, 0); y <- mul(-1, 0); v <- list(a, b, c, d, 1 / z, 1 / y) }; v }; \
         t3 <- function() { for (k in 1:2) v <- br(NaN); v }; \
         t4 <- function() { a <- NA_real_; b <- NaN; z <- -0; v <- NULL; \
                            for (k in 1:2) { p <- id(a + 1); q <- id(b + 1); m <- id(a - b); n <- id(b - a); \
                                             s <- id(z * 1); v <- list(e(p), e(q), e(m), e(n), 1 / s) }; v }",
    );
    let mut runs = 0;
    for src in ["t1()", "t2()", "t3()", "t4()"] {
        let (on, off, n) = both(&mut r, src);
        assert_eq!(on, off, "{src}");
        runs += n;
    }
    assert!(runs > 60, "inline runs: {runs}");
}
