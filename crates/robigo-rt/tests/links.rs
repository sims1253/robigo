//! Link counts of pending values during a complex assignment (C1c,
//! `crate::links`; GNU R's `INCLNK_stack_commit`, eval.c:997-1016): what
//! GNU R's node stack holds is linked, and nothing else.
//!
//! The R-level case is `tests/rt/assign/link-walk.R` (a builtin's C local,
//! `vapply`'s `X`). This file covers what no ported builtin shows from R
//! yet: a builtin's own arguments, which GNU R links while it evaluates the
//! later ones and drops before the call (`evalList`, byte code's
//! `BUILTIN_CALL_FRAME_ARGS`). GNU R's `rapply`, a builtin that walks its
//! argument across `FUN`'s calls, shows the effect on the 4.6.2 oracle,
//! JIT on and off, and byte-compiled with `compiler::cmpfun`:
//!
//! ```r
//! x <- list(1, 2, 3)
//! f <- function(e) { x[[3]] <<- 99; e }
//! g <- function(...) .Internal(rapply(x, f, "ANY", NULL, "list"))
//! unlist(g())  # 1 2 99: x was written in place
//! ```
//!
//! `hold(x, f)` below is that shape: an `.Internal` builtin (its arguments
//! in the caller's registers) that calls `f()` and then reads its own `x`.

mod common;

use common::*;
use robigo_rt::Profile;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `.Internal(hold(x, f))`: `f()`, then `x[[3]]` of the `x` it was given.
fn hold(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let none = s.args(cx, &[], None);
        let o = s.out(cx);
        cx.apply(call, args.root(1), none, env, o)?;
        Ok(())
    })?;
    let x = args.get(cx, 0);
    let third = x.as_vector::<robigo_rtcore::Ref<'_>>().expect("hold: a list").get(2);
    out.set(cx, third);
    Ok(())
}

fn session(p: Profile) -> Session {
    let mut r = Session::new(p);
    // eval column 11: an `.Internal` builtin, arguments evaluated
    robigo_rt::define_builtin(r.cx(), "hold", hold, 11, 2);
    r
}

/// A builtin's arguments hold no links while it runs: `f`'s `x[[3]] <<- 99`
/// on an `x` only its binding holds is written in place, and the builtin
/// sees it, as GNU R's `rapply` does.
#[test]
fn a_running_builtins_arguments_are_not_linked() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = session(p);
        r.val("x <- list(1, 2, 3); f <- function() x[[3]] <<- 99; g <- function() .Internal(hold(x, f))");
        assert_eq!(r.val("g()"), real(99.0), "{p:?}");
        // shared before the call: copied, the builtin keeps the old value
        r.val("x <- list(1, 2, 3); x2 <- x");
        assert_eq!(r.val("g()"), real(3.0), "{p:?}");
    }
}

/// A value pending in the caller while an argument is still being
/// evaluated is linked (`evalList`'s `INCREMENT_LINKS`, byte code's
/// pending operands): the assignment copies, the call sees the old value.
#[test]
fn a_pending_operand_is_linked() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut r = session(p);
        r.val("x <- c(1, 2); f <- function() { x[1] <<- 5; 0 }");
        assert_eq!(r.val("x + f()"), Val::Real(vec![1.0, 2.0]), "{p:?}");
        assert_eq!(r.val("x"), Val::Real(vec![5.0, 2.0]), "{p:?}");
    }
}
