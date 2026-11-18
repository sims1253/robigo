//! Deparse forces the promises it meets, as GNU R's `deparse2buff` does
//! (`case PROMSXP`: `eval(s, R_EmptyEnv)` unless `DELAYPROMISES`): in the
//! order its walk meets them, once each, an error in one raised; with
//! `delayPromises` none is forced. R code cannot make a raw promise, so a
//! builtin here does (`mkprom(expr)`: a promise of `expr` in the global
//! environment). These run without base's R code.

mod common;

use common::*;
use robigo_rt::Profile;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `mkprom(expr)`: an unforced promise of `expr` in the global environment.
fn mkprom(cx: &mut Cx, _call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.build(out, |cx| cx.mk_promise(args.get(cx, 0), cx.global_env()))
}

fn session(p: Profile) -> Session {
    let mut s = Session::new(p);
    robigo_rt::define_builtin(s.cx(), "mkprom", mkprom, 1, 1);
    s
}

fn strs(v: &[&str]) -> Val {
    Val::Str(v.iter().map(|s| s.to_string()).collect())
}

#[test]
fn deparse_forces_the_promises_it_meets_in_order() {
    for p in [Profile::Eval, Profile::Bc] {
        let mut s = session(p);
        s.val("n <- 0");
        s.val(r#"l <- list(mkprom(quote({ n <- n + 1; 1 + 2 })), mkprom(quote({ n <- n * 10; "b" })))"#);
        // deparse(l): both forced, the first first (n: 0 + 1, then * 10)
        assert_eq!(s.val(".Internal(deparse(l, 60L, TRUE, 0L, -1L))"), strs(&[r#"list(3, "b")"#]), "{p:?}");
        assert_eq!(s.val("n"), Val::Real(vec![10.0]), "{p:?}");
        // forced once: a second deparse runs no code
        assert_eq!(s.val(".Internal(deparse(l, 60L, TRUE, 0L, -1L))"), strs(&[r#"list(3, "b")"#]), "{p:?}");
        assert_eq!(s.val("n"), Val::Real(vec![10.0]), "{p:?}");
        // a promise inside a call, printed or deparsed
        s.val("cl <- as.call(list(quote(f), mkprom(quote({ n <- n + 5; 7 }))))");
        assert_eq!(s.val(".Internal(deparse(cl, 60L, TRUE, 0L, -1L))"), strs(&["f(7)"]), "{p:?}");
        assert_eq!(s.val("n"), Val::Real(vec![15.0]), "{p:?}");
        // delayPromises: none forced, its expression shown
        s.val("d <- list(mkprom(quote(undefined_thing)))");
        assert_eq!(
            s.val(".Internal(deparse(d, 60L, TRUE, 32L, -1L))"),
            strs(&["list(<promise: undefined_thing>)"]),
            "{p:?}"
        );
        // an error while forcing is raised
        assert!(
            s.err(".Internal(deparse(d, 60L, TRUE, 0L, -1L))").contains("object 'undefined_thing' not found"),
            "{p:?}"
        );
    }
}
