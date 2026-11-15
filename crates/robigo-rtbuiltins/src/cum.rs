//! GNU R's cum.c: `cumsum`, `cumprod`, `cummax` and `cummin` (`do_cum`,
//! names.c codes 1 to 4), the `Math` group's cumulative members.
//!
//! A double sum or product accumulates in `long double` (x87 extended
//! precision, robigo-numerics' `LDouble`), each step rounded to double, as
//! GNU R's `LDOUBLE` loop does; integer and logical input stays integer
//! (but for `cumprod`), with NA from the first NA or overflow on.

use crate::attrib::{Attr, Attrs};
use crate::util::not_yet;
use robigo_numerics::ldouble::LDouble as LD;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Cx, Dispatched, Generic, Out, RResult, Root};

const NAME: [&str; 4] = ["cumsum", "cumprod", "cummax", "cummin"];

const GENERIC: [Generic; 4] = [
    Generic::group("Math", "cumsum"),
    Generic::group("Math", "cumprod"),
    Generic::group("Math", "cummax"),
    Generic::group("Math", "cummin"),
];

/// `cumsum(x)` (`CODE` 1), `cumprod` (2), `cummax` (3), `cummin` (4).
// R: cum.c:do_cum
pub fn do_cum<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let k = (CODE - 1) as usize;
    crate::registry::by_name(NAME[k]).unwrap().check_arity(cx, call, args)?;
    let Dispatched::Default(out) = crate::math::math_dispatch(cx, &GENERIC[k], call, args, env, out)? else {
        return Ok(());
    };
    let xt = args.get(cx, 0).sexptype();
    if xt == CPLXSXP {
        return Err(not_yet(cx, Some(call), &format!("{}() of a complex vector (ccumsum, ccumprod)", NAME[k])));
    }
    let int = matches!(xt, INTSXP | LGLSXP) && CODE != 2;
    cx.scope(|cx, s| {
        // coerceVector(CAR(args), INTSXP or REALSXP), with its warnings
        let x = args.root(0);
        let t =
            s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, x, if int { INTSXP } else { REALSXP }, o))?;
        if t.bind(cx).is_package_altrep() {
            return Err(not_yet(cx, Some(call), &format!("{}() of a package ALTREP vector", NAME[k])));
        }
        let overflow = cx.build_with(out, |cx| {
            let tv = t.bind(cx);
            let names = crate::attrib::names(tv);
            let mut a = Attrs::new();
            if let Some(nm) = names {
                a.install(Attr { tag: cx.install(b"names")?, value: nm });
            }
            if int {
                let xs = tv.as_ints().expect("robigo: coerced to integer");
                let mut s = cx.alloc_with(xs.len(), |_| i32::MIN)?;
                let mut overflow = false;
                if !xs.is_empty() {
                    let is = s.as_mut_slice();
                    match CODE {
                        1 => overflow = icumsum(xs, is),
                        3 => icummax(xs, is),
                        _ => icummin(xs, is),
                    }
                }
                a.apply(cx, &mut s)?;
                Ok(Built::done(s.finish()).with(overflow))
            } else {
                let xs = tv.as_vector::<f64>().expect("robigo: coerced to double").as_slice().unwrap();
                let mut s = cx.alloc_with(xs.len(), |_| 0.0)?;
                if !xs.is_empty() {
                    let rs = s.as_mut_slice();
                    match CODE {
                        1 => cumsum(xs, rs),
                        2 => cumprod(xs, rs),
                        3 => cummax(xs, rs),
                        _ => cummin(xs, rs),
                    }
                }
                a.apply(cx, &mut s)?;
                Ok(Built::done(s.finish()).with(false))
            }
        })?;
        if overflow {
            cx.warning("integer overflow in 'cumsum'; use 'cumsum(as.numeric(.))'")?;
        }
        Ok(())
    })
}

/// `handleNaN(x, s)`: from the first NaN of `x` on, the result is NaN, or
/// NA from the first NA on (keeping the two apart).
// R: cum.c:handleNaN
fn handle_nan(x: &[f64], s: &mut [f64]) {
    let (mut has_na, mut has_nan) = (false, false);
    for (i, &v) in x.iter().enumerate() {
        has_nan = has_nan || v.is_nan();
        has_na = has_na || (has_nan && robigo_text::is_na(v));
        if has_na {
            s[i] = robigo_numerics::NA_REAL;
        } else if has_nan {
            s[i] = f64::NAN;
        }
    }
}

// R: cum.c:cumsum
fn cumsum(x: &[f64], s: &mut [f64]) {
    let sum = LD::cumsum_from(LD::ZERO, x, s);
    if sum.is_nan() {
        handle_nan(x, s);
    }
}

// R: cum.c:cumprod
fn cumprod(x: &[f64], s: &mut [f64]) {
    let prod = LD::cumprod_from(LD::ONE, x, s);
    if prod.is_nan() {
        handle_nan(x, s);
    }
}

// R: cum.c:cummax
fn cummax(x: &[f64], s: &mut [f64]) {
    let mut max = f64::NEG_INFINITY;
    for (i, &v) in x.iter().enumerate() {
        if v.is_nan() {
            return handle_nan(x, s);
        }
        max = if max > v { max } else { v };
        s[i] = max;
    }
}

// R: cum.c:cummin
fn cummin(x: &[f64], s: &mut [f64]) {
    let mut min = f64::INFINITY;
    for (i, &v) in x.iter().enumerate() {
        if v.is_nan() {
            return handle_nan(x, s);
        }
        min = if min < v { min } else { v };
        s[i] = min;
    }
}

/// `icumsum(x, s)`, `s` all NA: true when the sum left the integer range
/// (GNU R's warning, signalled by the caller).
// R: cum.c:icumsum
fn icumsum(x: &[i32], s: &mut [i32]) -> bool {
    let mut sum = 0.0f64;
    for (i, &v) in x.iter().enumerate() {
        if v == i32::MIN {
            break;
        }
        sum += v as f64;
        if sum > i32::MAX as f64 || sum < 1.0 + i32::MIN as f64 {
            return true;
        }
        s[i] = sum as i32;
    }
    false
}

// R: cum.c:icummax
fn icummax(x: &[i32], s: &mut [i32]) {
    if x[0] == i32::MIN {
        return; // all NA
    }
    let mut max = x[0];
    s[0] = max;
    for i in 1..x.len() {
        if x[i] == i32::MIN {
            break;
        }
        max = if max > x[i] { max } else { x[i] };
        s[i] = max;
    }
}

// R: cum.c:icummin
fn icummin(x: &[i32], s: &mut [i32]) {
    let mut min = x[0];
    s[0] = min;
    for i in 1..x.len() {
        if x[i] == i32::MIN {
            break;
        }
        min = if min < x[i] { min } else { x[i] };
        s[i] = min;
    }
}
