//! memory.c's builtins, reading their arguments as GNU R 4.6.2 does.

mod common;

use common::*;
use robigo_rt::Profile;

/// `do_gctorture`: a logical `on` is `asRbool`'s, anything else
/// `asInteger`'s, and `R_gc_torture` leaves the state alone for NA or a
/// negative gap. GNU R 4.6.2 (the oracle image), through `gctorture(on)`:
/// `c(TRUE, FALSE)` is "'length = 2' in coercion to 'logical(1)'", `NA` and
/// `logical(0)` are "NA in coercion to boolean", both against the
/// internal's call; `-1`, `NA_real_` and a function return the previous
/// state and change nothing.
#[test]
fn gctorture_reads_on_as_gnu_r_does() {
    let mut s = Session::new(Profile::Eval);
    let e = s.err(".Internal(gctorture(c(TRUE, FALSE)))");
    assert_eq!(e, "[gctorture] 'length = 2' in coercion to 'logical(1)'");
    assert_eq!(s.err(".Internal(gctorture(NA))"), "[gctorture] NA in coercion to boolean");
    assert_eq!(s.err(".Internal(gctorture(NA[0]))"), "[gctorture] NA in coercion to boolean");
    assert_eq!(s.val(".Internal(gctorture(5L))"), Val::Lgl(vec![0]));
    assert_eq!(s.val(".Internal(gctorture(-1))"), Val::Lgl(vec![1]));
    assert_eq!(s.val(".Internal(gctorture(NA_real_))"), Val::Lgl(vec![1]));
    assert_eq!(s.val(".Internal(gctorture(function() 1))"), Val::Lgl(vec![1]));
    assert_eq!(s.val(".Internal(gctorture(FALSE))"), Val::Lgl(vec![1]));
    assert_eq!(s.val(".Internal(gctorture(FALSE))"), Val::Lgl(vec![0]));
}
