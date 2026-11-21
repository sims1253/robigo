//! eval.c's `.Internal(setNumMathThreads(n))` and
//! `.Internal(setMaxNumMathThreads(n))`: the isolate's `R_num_math_threads`
//! and `R_max_num_math_threads` (Defn.h: both 1 at startup). robigo-rt runs
//! no math in parallel, so nothing reads them but these setters.

use robigo_rtcore::{Args, Cx, Out, RResult, Root};
use std::cell::Cell;

/// `R_num_math_threads`, `R_max_num_math_threads`.
struct MathThreads(Cell<(i32, i32)>);

fn get(cx: &Cx) -> (i32, i32) {
    match cx.ext::<MathThreads>() {
        Some(t) => t.0.get(),
        // R: Defn.h, INI_as(1)
        None => (1, 1),
    }
}

fn set(cx: &mut Cx, v: (i32, i32)) {
    match cx.ext::<MathThreads>() {
        Some(t) => t.0.set(v),
        None => cx.set_ext(MathThreads(Cell::new(v))),
    }
}

/// `.Internal(setNumMathThreads(n))`: the old count; `n` becomes the count
/// when it is in `0..=R_max_num_math_threads`.
// R: eval.c:do_setnumthreads
pub fn do_setnumthreads(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("setNumMathThreads", true).check_arity(cx, call, args)?;
    let old = get(cx).0;
    let new = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    // (read after the coercion, whose warning may run R code, as GNU R does)
    let max = get(cx).1;
    if new >= 0 && new <= max {
        set(cx, (new, max));
    }
    cx.build_scalar(out, old)
}

/// `.Internal(setMaxNumMathThreads(n))`: the old maximum; a nonnegative
/// `n` becomes the maximum, the count lowered to it.
// R: eval.c:do_setmaxnumthreads
pub fn do_setmaxnumthreads(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("setMaxNumMathThreads", true).check_arity(cx, call, args)?;
    let old = get(cx).1;
    let new = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    if new >= 0 {
        let num = get(cx).0;
        set(cx, (num.min(new), new));
    }
    cx.build_scalar(out, old)
}
