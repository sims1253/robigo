//! arithmetic.c's mathematical functions of one argument that the base
//! subset reaches: `sqrt` (`do_math1` with `math1`) and `abs` (`do_abs`).
//! Both are "Math" group generics.

use crate::attrib::Attrs;
use crate::util::{is_numeric, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Cx, Diag, Dispatched, Generic, Out, RResult, Root, View};

const NONNUM: &str = "non-numeric argument to mathematical function";

/// `DispatchGroup("Math", ...)` for an object argument.
fn math_dispatch<'o>(
    cx: &mut Cx,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    if !args.is_empty() && args.get(cx, 0).is_object() {
        cx.try_dispatch(g, call, args, env, out)
    } else {
        Ok(Dispatched::Default(out))
    }
}

/// `Rsqrt(x)`: exact for the squares of 1..11, `sqrt(-0) = -0`.
// R: arithmetic.c:Rsqrt
fn rsqrt(x: f64) -> f64 {
    if x == 0.0 {
        return x;
    }
    for i in 1..12 {
        if x == (i * i) as f64 {
            return i as f64;
        }
    }
    x.sqrt()
}

/// `math1(sa, f, lcall)`: `f` over the double coercion of a numeric
/// vector, its attributes kept, "NaNs produced" when a NaN appears from a
/// number.
// R: arithmetic.c:math1
fn math1(cx: &mut Cx, call: Root<'_>, x: Root<'_>, f: fn(f64) -> f64, out: Out<'_>) -> RResult<()> {
    if !is_numeric(x.bind(cx)) {
        return Err(cx.error_call(Some(call), NONNUM));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "a math function of a package ALTREP vector"));
    }
    let naflag = cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        let (vals, a): (Vec<f64>, Attrs<'_>) = {
            let mut a = Attrs::new();
            a.copy_all(xv);
            let v: Vec<f64> = match xv.view() {
                View::Real(v) => v.as_slice().unwrap().to_vec(),
                View::Int(v) => v
                    .as_slice()
                    .unwrap()
                    .iter()
                    .map(|&i| if i == i32::MIN { robigo_text::NA_REAL } else { i as f64 })
                    .collect(),
                View::Lgl(v) => v
                    .as_slice()
                    .unwrap()
                    .iter()
                    .map(|i| if i.0 == i32::MIN { robigo_text::NA_REAL } else { i.0 as f64 })
                    .collect(),
                _ => return Ok(Built::fail(Diag::error_call(Some(call), NONNUM))),
            };
            (v, a)
        };
        let mut naflag = false;
        let mut r = cx.alloc_with(vals.len(), |i| {
            let x = vals[i];
            let y = f(x);
            if y.is_nan() {
                if x.is_nan() {
                    x
                } else {
                    naflag = true;
                    y
                }
            } else {
                y
            }
        })?;
        a.apply(cx, &mut r)?;
        Ok(Built::done(r.finish()).with(naflag))
    })?;
    if naflag {
        cx.warning_call(Some(call), "NaNs produced")?;
    }
    Ok(())
}

const SQRT: Generic = Generic::group("Math", "sqrt");

/// `sqrt(x)` (`do_math1`, code 3).
// R: arithmetic.c:do_math1
pub fn do_sqrt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("sqrt").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = math_dispatch(cx, &SQRT, call, args, env, out)? else { return Ok(()) };
    if args.get(cx, 0).sexptype() == CPLXSXP {
        return Err(not_yet(cx, Some(call), "sqrt() of a complex vector (complex_math1)"));
    }
    math1(cx, call, args.root(0), rsqrt, out)
}

const ABS: Generic = Generic::group("Math", "abs");

/// `abs(x)`: integer for integer or logical input, double for double.
// R: arithmetic.c:do_abs
pub fn do_abs(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("abs").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = math_dispatch(cx, &ABS, call, args, env, out)? else { return Ok(()) };
    let x = args.root(0);
    match x.bind(cx).sexptype() {
        INTSXP | LGLSXP | REALSXP => {}
        CPLXSXP => return Err(not_yet(cx, Some(call), "abs() of a complex vector (do_cmathfuns)")),
        _ => return Err(cx.error_call(Some(call), NONNUM)),
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "abs() of a package ALTREP vector"));
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let mut a = Attrs::new();
        a.copy_all(xv);
        match xv.view() {
            View::Real(v) => {
                let v = v.as_slice().unwrap();
                let mut r = cx.alloc_with(v.len(), |i| v[i].abs())?;
                a.apply(cx, &mut r)?;
                Ok(r.finish().into())
            }
            _ => {
                let v = xv.as_ints().unwrap();
                let mut r = cx.alloc_with(v.len(), |i| if v[i] == i32::MIN { v[i] } else { v[i].abs() })?;
                a.apply(cx, &mut r)?;
                Ok(r.finish().into())
            }
        }
    })
}
