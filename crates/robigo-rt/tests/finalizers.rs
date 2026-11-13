//! Finalizers and dead roots (docs/g3.md §2.3, last bullet; §3 item 7; the
//! D9 bar): an object whose last reference is a VM register that is no
//! longer used must be finalized at the same `gc()` as in GNU R, and the
//! evaluation-count poll runs pending finalizers in GNU R's unit.

mod common;

use common::*;
use robigo_rt::Profile;

/// GNU R 4.6.2 (the oracle image), for
/// `f <- function() { reg.finalizer(new.env(), function(e) cat("finalized\n")); gc(); cat("after gc\n") }; invisible(f())`:
///
/// ```text
/// finalized
/// after gc
/// ```
///
/// Here `reg.finalizer` is a builtin, so the environment is an argument
/// register of the call: dead once the call returns, and cleared then.
#[test]
fn a_dead_register_does_not_delay_a_finalizer() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = Session::new(p);
        r.val(
            "f <- function() {
               reg.finalizer(new.env(), function(e) record(\"finalized\"))
               gc()
               record(\"after gc\")
             }
             f()",
        );
        assert_eq!(log_take(), vec!["finalized", "after gc"], "profile {p:?}");
    }
}

/// The same with the object as a discarded statement's value and as an
/// arithmetic operand's: each register is cleared at its last use.
#[test]
fn statement_values_and_operands_are_cleared_at_last_use() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = Session::new(p);
        r.val(
            "mk <- function() { e <- new.env(); reg.finalizer(e, function(e) record(\"fin\")); e }
             f <- function() {
               keep(mk())
               gc()
               record(\"after\")
             }
             f()",
        );
        assert_eq!(log_take(), vec!["fin", "after"], "profile {p:?}");
    }
}

/// A loop body's value lands in a register no later statement reuses: it
/// is cleared at its last use, so the gc() after the loop finalizes it
/// (GNU R's `do_for` keeps no reference to the body's value).
#[test]
fn a_loop_bodys_dead_value_is_cleared() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = Session::new(p);
        r.val(
            "mk <- function() { e <- new.env(); reg.finalizer(e, function(e) record(\"fin\")); e }
             f <- function() {
               for (i in 1:1) keep(mk())
               gc()
               record(\"after\")
             }
             f()",
        );
        assert_eq!(log_take(), vec!["fin", "after"], "profile {p:?}");
    }
}

/// A live register keeps its object: the finalizer waits for the gc()
/// after the last use.
#[test]
fn a_live_binding_keeps_its_object() {
    let mut r = Session::new(Profile::Eval);
    r.val(
        "f <- function() {
           e <- new.env()
           reg.finalizer(e, function(e) record(\"fin\"))
           gc()
           record(\"between\")
           e <- NULL
           gc()
           record(\"end\")
         }
         f()",
    );
    assert_eq!(log_take(), vec!["between", "fin", "end"]);
}

/// Pending finalizers run at the evaluation-count poll (eval.c:1096):
/// after an automatic collection made one pending, it runs within the
/// next 1001 evaluations, before the loop ends.
#[test]
fn the_eval_count_poll_runs_pending_finalizers() {
    let mut r = Session::new(Profile::Eval);
    r.val(
        "mk <- function() { reg.finalizer(new.env(), function(e) record(\"fin\")); NULL }
         mk()",
    );
    // a collection that finds the environment dead, with no gc() call to
    // run its finalizer
    r.cx().collect();
    assert!(r.cx().finalizers_pending());
    r.val("n <- 0; while (n < 400) n <- n + 1; record(\"done\")");
    assert_eq!(log_take(), vec!["fin", "done"]);
}

/// A `break` or `next` abandons the temporaries of the expression it is in:
/// they are cleared, lexically and when the jump lands through a promise
/// (GNU R keeps no reference to the abandoned operand).
#[test]
fn break_and_next_clear_the_temporaries_they_abandon() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = Session::new(p);
        r.val("mk <- function() { e <- new.env(); reg.finalizer(e, function(e) record(\"fin\")); e }; brk <- function(x) x");
        r.val("f <- function() { repeat { keep(mk()) + { break } }; gc(); record(\"after\") }; f()");
        assert_eq!(log_take(), vec!["fin", "after"], "lexical break, profile {p:?}");
        r.val("f <- function() { repeat { keep(mk()) + brk(break) }; gc(); record(\"after\") }; f()");
        assert_eq!(log_take(), vec!["fin", "after"], "break through a promise, profile {p:?}");
        r.val("f <- function() { i <- 0; while (i < 1) { i <- i + 1; keep(mk()) + { next } }; gc(); record(\"after\") }; f()");
        assert_eq!(log_take(), vec!["fin", "after"], "lexical next, profile {p:?}");
    }
}

/// A lexical `next` is a back-edge: in the `Bc` profile it polls as
/// bcEval's GOTO does, so a loop that only ever takes `next` still runs
/// pending finalizers.
#[test]
fn next_polls_like_a_back_edge() {
    let mut r = Session::new(Profile::Bc);
    r.val("mk <- function() { reg.finalizer(new.env(), function(e) record(\"fin\")); NULL }; mk()");
    r.cx().collect();
    assert!(r.cx().finalizers_pending());
    r.val("n <- 0; repeat { n <- n + 1; if (n > 3000) break; next }; record(\"done\")");
    assert_eq!(log_take(), vec!["fin", "done"]);
}
