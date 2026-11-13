//! The evaluator on R source: constants, variables, control flow, calls,
//! promises, `...`, errors with GNU R's messages, both profiles.

mod common;

use common::*;
use robigo_rt::Profile;

fn eval_session() -> Session {
    Session::new(Profile::Eval)
}

#[test]
fn constants_and_arithmetic() {
    let mut r = eval_session();
    assert_eq!(r.val("1"), real(1.0));
    assert_eq!(r.val("1 + 2"), real(3.0));
    assert_eq!(r.val("2L * 3L"), int(6));
    assert_eq!(r.val("7 %/% 2"), real(3.0));
    assert_eq!(r.val("-7 %% 3"), real(2.0));
    assert_eq!(r.val("2 ^ 10"), real(1024.0));
    assert_eq!(r.val("1L / 2L"), real(0.5));
    assert_eq!(r.val("TRUE + TRUE"), int(2));
    assert_eq!(r.val("-(3L)"), int(-3));
    assert_eq!(r.val("1 < 2"), lgl(true));
    assert_eq!(r.val("2L == 2"), lgl(true));
    assert_eq!(r.val("!TRUE"), lgl(false));
    assert_eq!(r.val("NA_integer_ + 1L"), int(NA));
    assert_eq!(r.val("\"bc\" == \"bc\""), lgl(true));
    assert_eq!(r.val("\"bc\" != \"b\""), lgl(true));
    assert_eq!(r.val("(1 + 2) * 3"), real(9.0));
    assert_eq!(r.val("TRUE & NA"), lgl_na());
    assert_eq!(r.val("FALSE & NA"), lgl(false));
}

fn lgl_na() -> Val {
    Val::Lgl(vec![NA])
}

#[test]
fn variables_and_assignment() {
    let mut r = eval_session();
    assert_eq!(r.val("x <- 5; x + 1"), real(6.0));
    assert_eq!(r.val("y = 2; y * x"), real(10.0));
    assert_eq!(r.val("\"z\" <- 3; z"), real(3.0));
    let (v, vis) = r.run("w <- 4").unwrap();
    assert_eq!((v, vis), (real(4.0), false));
    let (v, vis) = r.run("(w <- 4)").unwrap();
    assert_eq!((v, vis), (real(4.0), true));
    assert_eq!(r.err("nope"), "object 'nope' not found");
}

#[test]
fn if_and_braces() {
    let mut r = eval_session();
    assert_eq!(r.val("if (TRUE) 1 else 2"), real(1.0));
    assert_eq!(r.val("if (FALSE) 1 else 2"), real(2.0));
    let (v, vis) = r.run("if (FALSE) 1").unwrap();
    assert_eq!((v, vis), (Val::Null, false));
    assert_eq!(r.val("{ 1; 2; 3 }"), real(3.0));
    assert_eq!(r.val("{}"), Val::Null);
    assert_eq!(r.err("if (NA) 1"), "[if] missing value where TRUE/FALSE needed");
    assert_eq!(r.err("if (NA_real_) 1"), "[if] argument is not interpretable as logical");
    assert_eq!(r.err("if (NULL) 1"), "[if] argument is of length zero");
    assert_eq!(r.err("if (1:2) 1"), "[if] the condition has length > 1");
    assert_eq!(r.val("if (\"TRUE\") 1 else 2"), real(1.0));
    assert_eq!(r.val("if (2L) \"a\" else \"b\""), str1("a"));
}

#[test]
fn closures_and_fib() {
    let mut r = eval_session();
    assert_eq!(r.val("f <- function(x) x * 2; f(21)"), real(42.0));
    assert_eq!(r.val("fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2); fib(20)"), real(6765.0));
    let (v, vis) = r.run("g <- function() { a <- 1 }; g()").unwrap();
    assert_eq!((v, vis), (real(1.0), false));
}

#[test]
fn calls_rows() {
    // benchmarks/suite/calls.R's closures without S3 and tryCatch
    let mut r = eval_session();
    r.val(
        "scale_by <- function(x, scale = 1, label = \"a\") { if (label == \"zz\") x else x * scale }
         weigh <- function(x, y) { z <- x * y; z + 1 }
         tag <- function(x, label) { if (label == \"bc\") x * 2 else x }
         acct <- function() {
           bal <- 0
           dep <- function(x) { bal <<- bal + x; bal }
           get <- function() bal
           dep(3); dep(4)
           get()
         }",
    );
    assert_eq!(r.val("scale_by(5)"), real(5.0));
    assert_eq!(r.val("scale_by(5, 3)"), real(15.0));
    assert_eq!(r.val("scale_by(5, label = \"zz\", scale = 9)"), real(5.0));
    assert_eq!(r.val("weigh(y = 2, x = 3)"), real(7.0));
    assert_eq!(r.val("tag(1.5, \"bc\")"), real(3.0));
    assert_eq!(r.val("tag(1.5, \"b\")"), real(1.5));
    assert_eq!(r.val("acct()"), real(7.0));
    // the loop of calls.R (S3 and tryCatch rows left out)
    assert_eq!(
        r.val(
            "s <- 0
             for (i in 1:100) {
               s <- s + scale_by(i * 0.5) + weigh(y = 2, x = i) + tag(i * 0.25, \"bc\")
             }
             s"
        ),
        real((1..=100).map(|i| i as f64 * 0.5 + (2 * i + 1) as f64 + i as f64 * 0.5).sum())
    );
}

#[test]
fn partial_matching_and_errors() {
    let mut r = eval_session();
    r.val("f <- function(alpha, beta) alpha - beta");
    assert_eq!(r.val("f(be = 1, al = 3)"), real(2.0));
    assert_eq!(r.err("f(alpha = 1, alpha = 2)"), "[f] formal argument \"alpha\" matched by multiple actual arguments");
    assert_eq!(r.err("f(1)"), "[f] argument \"beta\" is missing, with no default");
    r.val("g <- function(x) missing(x)");
    assert_eq!(r.val("g()"), lgl(true));
    assert_eq!(r.val("g(1)"), lgl(false));
    r.val("h <- function(y) g(y)");
    assert_eq!(r.val("h()"), lgl(true));
}

#[test]
fn dots() {
    let mut r = eval_session();
    r.val("g <- function(a, b) a - b; f <- function(...) g(...)");
    assert_eq!(r.val("f(5, 3)"), real(2.0));
    assert_eq!(r.val("f(b = 5, 3)"), real(-2.0));
    r.val("d2 <- function(...) ..2");
    assert_eq!(r.val("d2(1, 2, 3)"), real(2.0));
    assert_eq!(r.err("d2(1)"), "[d2] the ... list contains fewer than 2 elements");
    r.val("plus <- function(...) `+`(...)");
    assert_eq!(r.val("plus(2, 3)"), real(5.0));
    assert_eq!(r.err("f2 <- function() g(...); f2()"), "[f2] '...' used in an incorrect context");
}

#[test]
fn loops() {
    let mut r = eval_session();
    assert_eq!(r.val("s <- 0; for (i in 1:10) s <- s + i; s"), real(55.0));
    assert_eq!(r.val("i"), int(10));
    let (v, vis) = r.run("for (i in 1:3) i").unwrap();
    assert_eq!((v, vis), (Val::Null, false));
    assert_eq!(r.val("n <- 0; while (n < 7) n <- n + 1; n"), real(7.0));
    assert_eq!(r.val("n <- 0; repeat { n <- n + 1; if (n > 4) break }; n"), real(5.0));
    assert_eq!(r.val("s <- 0; for (i in 1:10) { if (i %% 2 == 0) next; s <- s + i }; s"), real(25.0));
    assert_eq!(r.val("first <- function(v) { for (x in v) if (x > 2) return(x); 0 }; first(1:5)"), int(3));
    assert_eq!(r.val("first(1:2)"), real(0.0));
    // break and next through a promise land at the loop's context
    r.val("brk <- function(x) x");
    assert_eq!(r.val("n <- 0; for (i in 1:5) { n <- n + 1; brk(if (i == 3) break) }; n"), real(3.0));
    assert_eq!(r.val("n <- 0; for (i in 1:5) { brk(if (i %% 2 == 0) next); n <- n + 1 }; n"), real(3.0));
    assert_eq!(r.err("for (i in quote(f(x))) 1"), "[for] invalid for() loop sequence");
}

#[test]
fn return_from_a_promise_leaves_its_function() {
    let mut r = eval_session();
    r.val("id <- function(x) x; f <- function() { id(return(5)); 6 }");
    assert_eq!(r.val("f()"), real(5.0));
    assert_eq!(r.err("return(1)"), "no function to return from, jumping to top level");
}

#[test]
fn quote_function_and_logic() {
    let mut r = eval_session();
    assert_eq!(r.val("quote(x + y)"), Val::Other(6));
    assert_eq!(r.val("quote(x)"), Val::Other(1));
    assert_eq!(r.val("TRUE && FALSE"), lgl(false));
    assert_eq!(r.val("FALSE || TRUE"), lgl(true));
    assert_eq!(r.val("NA && FALSE"), lgl(false));
    assert_eq!(r.val("NA || TRUE"), lgl(true));
    assert_eq!(r.val("FALSE && stop_never_called()"), lgl(false));
    assert_eq!(r.err("\"a\" && TRUE"), "[&&] invalid 'x' type in 'x && y'");
    assert_eq!(r.err("FALSE || 1:2"), "[||] 'length = 2' in coercion to 'logical(1)'");
    assert_eq!(r.val("(function(x, y = x * 2) y)(4)"), real(8.0));
}

#[test]
fn a_users_if_replaces_bases_in_the_eval_profile() {
    let mut r = eval_session();
    r.val("f <- function() if (TRUE) 1 else 2");
    assert_eq!(r.val("f()"), real(1.0));
    // GNU R's AST interpreter looks `if` up at every evaluation
    r.val("`if` <- function(c, a, b) \"mine\"");
    assert_eq!(r.val("f()"), str1("mine"));
    r.val("rm_if <- 1");
    r.val("`+` <- function(e1, e2) \"plus\"");
    assert_eq!(r.val("1 + 2"), str1("plus"));
}

#[test]
fn promises_are_forced_once_in_order() {
    let mut r = eval_session();
    r.val("f <- function(a, b) { b; a; a }");
    r.val("f(record(\"a\"), record(\"b\"))");
    assert_eq!(log_take(), vec!["b", "a"]);
    r.val("g <- function(x) 1");
    r.val("g(record(\"never\"))");
    assert!(log_take().is_empty());
    assert_eq!(
        r.err("h <- function(x = x) x; h()"),
        "[h] promise already under evaluation: recursive default argument reference or earlier problems?"
    );
}

#[test]
fn deep_recursion_reports_gnu_rs_error() {
    let mut r = eval_session();
    r.cx().set_expressions(200);
    r.val("inf <- function(n) inf(n + 1)");
    assert_eq!(r.err("inf(1)"), "evaluation nested too deeply: infinite recursion / options(expressions=)?");
    r.cx().set_expressions(5000);
    assert_eq!(r.val("1"), real(1.0));
}

#[test]
fn the_bc_profile_runs_the_same_programs() {
    let mut r = Session::new(Profile::Bc);
    assert_eq!(r.val("fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2); fib(15)"), real(610.0));
    assert_eq!(r.val("s <- 0; for (i in 1:10) s <- s + i; s"), real(55.0));
    r.val("f <- function(x) missing(x)");
    assert_eq!(r.val("f()"), lgl(true));
}

/// GNU R's evaluation depth per evaluation point (`eval`'s
/// INCREMENT_EVAL_DEPTH for every symbol, call and promise): how many
/// levels a recursion reaches before "evaluation nested too deeply" is
/// GNU R 4.6.2's exactly. Recorded on the oracle image with the JIT off
/// (`compiler::enableJIT(0)`, the AST interpreter whose observables the
/// `Eval` profile keeps), `options(expressions = 200)`, at the top level:
///
/// ```text
/// f: 98   g: 66   h: 98   p: 98
/// ```
#[test]
fn eval_depth_matches_gnu_rs_ast_interpreter() {
    let mut r = eval_session();
    r.cx().set_expressions(200);
    let cases = [
        ("k <- 0; f <- function() { k <<- k + 1; f() }", "f()", 98.0),
        ("k <- 0; g <- function(n) { k <<- k + 1; if (n > 0) g(n + 1) else 0 }", "g(1)", 66.0),
        ("k <- 0; h <- function(x) { k <<- k + 1; h(x + 1) }", "h(1)", 98.0),
        ("k <- 0; p <- function(x) { k <<- k + 1; x; p(x + 1) }", "p(1)", 98.0),
    ];
    for (def, call, k) in cases {
        r.val(def);
        assert_eq!(r.err(call), "evaluation nested too deeply: infinite recursion / options(expressions=)?");
        assert_eq!(r.val("k"), real(k), "{call}");
    }
}

/// `eval`'s poll runs when its count passes 1000 (eval.c:1091), before the
/// evaluation that made it pass: a pending finalizer runs exactly there.
/// At the top level each `record("x")` is two evaluations (the call, its
/// argument), as in GNU R's REPL.
#[test]
fn the_eval_count_polls_in_gnu_rs_unit() {
    let mut r = eval_session();
    r.val("mk <- function() { reg.finalizer(new.env(), function(e) record(\"fin\")); NULL }; mk()");
    r.cx().collect();
    assert!(r.cx().finalizers_pending());
    // 990 evaluations done: the 11th evaluation from here is the 1001st
    r.cx().evalcount().set(990);
    r.val("record(\"1\"); record(\"2\"); record(\"3\"); record(\"4\"); record(\"5\"); record(\"6\")");
    assert_eq!(log_take(), vec!["1", "2", "3", "4", "5", "fin", "6"]);
}
