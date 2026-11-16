//! B2a: the attribute setters (`attr<-`, `attributes<-`, `class<-`,
//! `oldClass<-`, `unclass`, `dim<-`, `dimnames<-`, `names<-`, `levels<-`)
//! write their object in place exactly when GNU R 4.6.2's do, in both
//! profiles. Their values and messages are checked by `tests/rt/attrib`.

mod common;

use common::*;
use robigo_rt::Profile;

fn sessions() -> [Session; 2] {
    [Session::new(Profile::Eval), Session::new(Profile::Bc)]
}

/// Each step, run on a bound and otherwise unreferenced `x`, and whether it
/// leaves `x` at the same address: GNU R 4.6.2's answers, read from
/// `.Internal(inspect(x))` after each step at the top level with the JIT
/// off. `attributes<-` with attributes to set copies an object it may
/// write (`MAYBE_REFERENCED(object) && nattrs`), and `unclass` outside an
/// assignment copies a referenced one.
///
/// The test reads addresses, so under `ROBIGO_GC_MOVE=1` (every collection
/// moves objects) it skips and says so in the log. They are production
/// identities (docs/g3.md §15.3 E5), so it runs with `ROBIGO_NO_INPLACE`
/// off whatever the environment says; [`no_inplace_copies_every_setter`]
/// is the mode's side.
#[test]
fn setters_write_in_place_when_gnu_r_does() {
    if std::env::var("ROBIGO_GC_MOVE").as_deref() == Ok("1") {
        // to the stream itself: libtest captures `eprintln!` of a passing test
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "setters_write_in_place_when_gnu_r_does: skipped, under ROBIGO_GC_MOVE=1 an address does not identify an object"
        );
        return;
    }
    for mut s in sessions() {
        s.cx().set_no_inplace(false);
        run_steps(&mut s, |in_place| in_place);
    }
}

/// `ROBIGO_NO_INPLACE=1` (docs/g3.md §16): every setter step copies its
/// object, and the values are production's.
#[test]
fn no_inplace_copies_every_setter() {
    if std::env::var("ROBIGO_GC_MOVE").as_deref() == Ok("1") {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "no_inplace_copies_every_setter: skipped, under ROBIGO_GC_MOVE=1 an address does not identify an object"
        );
        return;
    }
    for mut s in sessions() {
        s.cx().set_no_inplace(true);
        run_steps(&mut s, |_| false);
    }
}

/// Each setter step on `x`, checking whether `x` stays at its address
/// against `expect(GNU R's answer)`, then the values.
fn run_steps(s: &mut Session, expect: impl Fn(bool) -> bool) {
    let steps: &[(&str, bool)] = &[
        ("attr(x, 'foo') <- 1", true),
        ("attributes(x) <- list(foo = 1)", false),
        ("attributes(x) <- NULL", true),
        ("class(x) <- 'bar'", true),
        ("oldClass(x) <- 'baz'", true),
        ("x <- unclass(x)", false),
        ("dim(x) <- c(2L, 2L)", true),
        ("dimnames(x) <- list(c('a', 'b'), NULL)", true),
        ("names(x) <- c('a', 'b', 'c', 'd')", true),
        ("levels(x) <- c('a', 'b')", true),
        ("y <- x; attr(x, 'foo') <- 2", false),
    ];
    s.val("x <- c(1, 2, 3, 4)");
    for &(src, in_place) in steps {
        if src.starts_with("names") {
            // the oracle's sequence drops the dim first
            s.val("dim(x) <- NULL");
        }
        s.val("a <- addr(x)");
        s.val(src);
        let want = expect(in_place);
        assert_eq!(s.val("addr(x) == a"), lgl(want), "{src}: in place {in_place} in GNU R, expected {want}");
    }
    // the copy took the attribute, the object it shared with did not
    assert_eq!(s.val("attr(x, 'foo')"), Val::Real(vec![2.0]));
    assert_eq!(s.val("attr(y, 'foo')"), Val::Null);
    assert_eq!(s.val("x"), Val::Real(vec![1.0, 2.0, 3.0, 4.0]));
}
