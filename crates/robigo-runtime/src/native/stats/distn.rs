//! The distribution functions of the stats package that are native
//! routines only (distn.c): the noncentral beta, F, t and chi-squared
//! families, Tukey's, Wilcoxon's and the signed rank distribution.
//!
//! distn.c is copy'n'paste of arithmetic.c's `math3`/`math4`
//! dispatchers (its own comment says "KEEP IN SYNC!"), so the argument
//! recycling, the NA/NaN rule and the attribute propagation are the ones
//! [`super::super::base`]'s math builtins use; the C file keeps its own
//! copy, and so does this port.

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;
use robigo_numerics as nmath;

const MSG_NA: &str = "NaNs produced";
const MSG_NONNUM_MATH: &str = "non-numeric argument to mathematical function";

fn is_numeric(v: &Value) -> bool {
    match v {
        Value::Int(_) => !v.inherits("factor"),
        Value::Lgl(_) | Value::Real(_) => true,
        _ => false,
    }
}

fn reals(v: &Value) -> Vec<f64> {
    crate::coerce::to_real(v, &mut Default::default())
}

fn copy_all_attrs(from: &Value, to: &mut Value) {
    if let Some(a) = from.attrs() {
        to.set_attrs(Some(Box::new(a.clone())));
    }
}

/// `asInteger()` of a flag argument: nonzero is true.
fn flag(v: &Value) -> bool {
    crate::coerce::as_integer(v) != 0
}

/// `if_NA_Math3_set` / `if_NA_Math4_set`: NA if any input is NA, NaN if
/// any input is NaN.
fn na_result(xs: &[f64]) -> Option<f64> {
    if xs.iter().any(|&x| is_na_real(x)) {
        Some(NA_REAL)
    } else if xs.iter().any(|x| x.is_nan()) {
        Some(f64::NAN)
    } else {
        None
    }
}

fn emit_nmath_warnings(i: &mut Interp) -> RResult<()> {
    for w in nmath::take_warnings() {
        let w = w.strip_suffix('\n').map(str::to_string).unwrap_or(w);
        i.warning(w)?;
    }
    Ok(())
}

/// R: distn.c:math3_1 / math3_2 (`SETUP_Math3` .. `FINISH_Math3`)
fn math3(i: &mut Interp, a: &[Arg], f: &dyn Fn(f64, f64, f64) -> f64) -> RResult<Value> {
    if a.len() < 3 {
        return Err(Flow::error(MSG_NONNUM_MATH));
    }
    let (sa, sb, sc) = (&a[0].value, &a[1].value, &a[2].value);
    if !is_numeric(sa) || !is_numeric(sb) || !is_numeric(sc) {
        return Err(Flow::error(MSG_NONNUM_MATH));
    }
    let (na, nb, nc) = (sa.len(), sb.len(), sc.len());
    if na == 0 || nb == 0 || nc == 0 {
        let mut sy = Value::reals(vec![]);
        if na == 0 {
            copy_all_attrs(sa, &mut sy);
        }
        return Ok(sy);
    }
    let n = na.max(nb).max(nc);
    let (va, vb, vc) = (reals(sa), reals(sb), reals(sc));
    nmath::take_warnings();
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for k in 0..n {
        let (ai, bi, ci) = (va[k % na], vb[k % nb], vc[k % nc]);
        let v = match na_result(&[ai, bi, ci]) {
            Some(v) => v,
            None => {
                let v = f(ai, bi, ci);
                if v.is_nan() {
                    naflag = true;
                }
                v
            }
        };
        y.push(v);
    }
    emit_nmath_warnings(i)?;
    if naflag {
        i.warning(MSG_NA)?;
    }
    let mut sy = Value::reals(y);
    if n == na {
        copy_all_attrs(sa, &mut sy);
    } else if n == nb {
        copy_all_attrs(sb, &mut sy);
    } else if n == nc {
        copy_all_attrs(sc, &mut sy);
    }
    Ok(sy)
}

/// R: distn.c:math4_1 / math4_2 (`SETUP_Math4` .. `FINISH_Math4`)
fn math4(i: &mut Interp, a: &[Arg], f: &dyn Fn(f64, f64, f64, f64) -> f64) -> RResult<Value> {
    if a.len() < 4 {
        return Err(Flow::error(MSG_NONNUM_MATH));
    }
    let (sa, sb, sc, sd) = (&a[0].value, &a[1].value, &a[2].value, &a[3].value);
    if !is_numeric(sa) || !is_numeric(sb) || !is_numeric(sc) || !is_numeric(sd) {
        return Err(Flow::error(MSG_NONNUM_MATH));
    }
    let (na, nb, nc, nd) = (sa.len(), sb.len(), sc.len(), sd.len());
    if na == 0 || nb == 0 || nc == 0 || nd == 0 {
        let mut sy = Value::reals(vec![]);
        if na == 0 {
            copy_all_attrs(sa, &mut sy);
        }
        return Ok(sy);
    }
    let n = na.max(nb).max(nc).max(nd);
    let (va, vb, vc, vd) = (reals(sa), reals(sb), reals(sc), reals(sd));
    nmath::take_warnings();
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for k in 0..n {
        let (ai, bi, ci, di) = (va[k % na], vb[k % nb], vc[k % nc], vd[k % nd]);
        let v = match na_result(&[ai, bi, ci, di]) {
            Some(v) => v,
            None => {
                let v = f(ai, bi, ci, di);
                if v.is_nan() {
                    naflag = true;
                }
                v
            }
        };
        y.push(v);
    }
    emit_nmath_warnings(i)?;
    if naflag {
        i.warning(MSG_NA)?;
    }
    let mut sy = Value::reals(y);
    if n == na {
        copy_all_attrs(sa, &mut sy);
    } else if n == nb {
        copy_all_attrs(sb, &mut sy);
    } else if n == nc {
        copy_all_attrs(sc, &mut sy);
    } else if n == nd {
        copy_all_attrs(sd, &mut sy);
    }
    Ok(sy)
}

// ----------------------------------------------------------------------
// The four-argument families (math4_1 with log, math4_2 with
// lower.tail/log.p)

pub fn dnbeta(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|x, a, b, ncp| nmath::nbeta::dnbeta(x, a, b, ncp, lg))
}

pub fn pnbeta(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(5).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|x, a, b, ncp| nmath::nbeta::pnbeta(x, a, b, ncp, lt, lg))
}

pub fn qnbeta(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(5).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|p, a, b, ncp| nmath::nbeta::qnbeta(p, a, b, ncp, lt, lg))
}

pub fn dnf(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|x, df1, df2, ncp| nmath::nf::dnf(x, df1, df2, ncp, lg))
}

pub fn pnf(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(5).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|x, df1, df2, ncp| nmath::nf::pnf(x, df1, df2, ncp, lt, lg))
}

pub fn qnf(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(5).map(|a| flag(&a.value)).unwrap_or(false);
    math4(i, a, &|p, df1, df2, ncp| nmath::nf::qnf(p, df1, df2, ncp, lt, lg))
}

// ----------------------------------------------------------------------
// The three-argument families (math3_1 with log, math3_2 with
// lower.tail/log.p)

pub fn dnt(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lg = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|x, df, ncp| nmath::nt::dnt(x, df, ncp, lg))
}

pub fn pnt(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|x, df, ncp| nmath::nt::pnt(x, df, ncp, lt, lg))
}

pub fn qnt(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|p, df, ncp| nmath::nt::qnt(p, df, ncp, lt, lg))
}

pub fn dnchisq(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lg = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|x, df, ncp| nmath::nchisq::dnchisq(x, df, ncp, lg))
}

pub fn pnchisq(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|x, df, ncp| nmath::nchisq::pnchisq(x, df, ncp, lt, lg))
}

pub fn qnchisq(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let lt = a.get(3).map(|a| flag(&a.value)).unwrap_or(false);
    let lg = a.get(4).map(|a| flag(&a.value)).unwrap_or(false);
    math3(i, a, &|p, df, ncp| nmath::nchisq::qnchisq(p, df, ncp, lt, lg))
}
