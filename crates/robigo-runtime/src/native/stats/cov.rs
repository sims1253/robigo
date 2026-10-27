//! `C_cov` and `C_cor`: covariance and correlation (port of GNU R's
//! src/library/stats/src/cov.c). Sums are accumulated in long double, as
//! in R, so results agree to the last bit.

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::nmath::ldouble::LDouble;
use crate::sym::s;
use crate::value::*;

type LD = LDouble;

fn ld(x: f64) -> LD {
    LD::from_f64(x)
}

fn ld_int(n: usize) -> LD {
    LD::from_f64(n as f64)
}

/// `x - y` with `x` a double and `y` a long double.
fn dm(x: f64, y: LD) -> LD {
    ld(x) - y
}

/// R's `sign()`.
fn sign(x: f64) -> f64 {
    if x.is_nan() {
        x
    } else if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn clamp_ld(x: LD) -> LD {
    if x >= ld(1.0) {
        ld(1.0)
    } else if x <= ld(-1.0) {
        ld(-1.0)
    } else {
        x
    }
}

fn clamp(x: f64) -> f64 {
    if x >= 1.0 {
        1.0
    } else if x <= -1.0 {
        -1.0
    } else {
        x
    }
}

struct Out {
    ans: Vec<f64>,
    ncx: usize,
    sd_0: bool,
}

impl Out {
    fn set(&mut self, i: usize, j: usize, v: f64) {
        self.ans[i + j * self.ncx] = v;
    }
    fn get(&self, i: usize, j: usize) -> f64 {
        self.ans[i + j * self.ncx]
    }
}

/// COV_PAIRWISE_BODY for columns `xx` and `yy`, stored at (i, j).
fn pairwise_body(o: &mut Out, i: usize, j: usize, xx: &[f64], yy: &[f64], cor: bool, kendall: bool) {
    let n = xx.len();
    let ok = |k: usize| !(xx[k].is_nan() || yy[k].is_nan());
    let mut nobs = 0usize;
    let (mut xmean, mut ymean) = (ld(0.0), ld(0.0));
    if !kendall {
        for k in 0..n {
            if ok(k) {
                nobs += 1;
                xmean += xx[k];
                ymean += yy[k];
            }
        }
    } else {
        nobs = (0..n).filter(|&k| ok(k)).count();
    }
    if nobs < 2 {
        o.set(i, j, NA_REAL);
        return;
    }
    let (mut xsd, mut ysd, mut sum) = (ld(0.0), ld(0.0), ld(0.0));
    let mut n1 = 0usize;
    if !kendall {
        xmean /= ld_int(nobs);
        ymean /= ld_int(nobs);
        n1 = nobs - 1;
    }
    for k in 0..n {
        if !ok(k) {
            continue;
        }
        if !kendall {
            let xm = dm(xx[k], xmean);
            let ym = dm(yy[k], ymean);
            sum += xm * ym;
            if cor {
                xsd += xm * xm;
                ysd += ym * ym;
            }
        } else {
            for m in 0..k {
                if ok(m) {
                    let xm = ld(sign(xx[k] - xx[m]));
                    let ym = ld(sign(yy[k] - yy[m]));
                    sum += xm * ym;
                    if cor {
                        xsd += xm * xm;
                        ysd += ym * ym;
                    }
                }
            }
        }
    }
    if cor {
        if xsd == ld(0.0) || ysd == ld(0.0) {
            o.sd_0 = true;
            sum = ld(NA_REAL);
        } else {
            if !kendall {
                xsd /= ld_int(n1);
                ysd /= ld_int(n1);
                sum /= ld_int(n1);
            }
            sum /= xsd.sqrt() * ysd.sqrt();
            sum = clamp_ld(sum);
        }
    } else if !kendall {
        sum /= ld_int(n1);
    }
    o.set(i, j, sum.to_f64());
}

fn col(x: &[f64], n: usize, i: usize) -> &[f64] {
    &x[i * n..(i + 1) * n]
}

fn cov_pairwise1(o: &mut Out, n: usize, ncx: usize, x: &[f64], cor: bool, kendall: bool) {
    for i in 0..ncx {
        for j in 0..=i {
            pairwise_body(o, i, j, col(x, n, i), col(x, n, j), cor, kendall);
            let v = o.get(i, j);
            o.set(j, i, v);
        }
    }
}

fn cov_pairwise2(o: &mut Out, n: usize, ncx: usize, ncy: usize, x: &[f64], y: &[f64], cor: bool, kendall: bool) {
    for i in 0..ncx {
        for j in 0..ncy {
            pairwise_body(o, i, j, col(x, n, i), col(y, n, j), cor, kendall);
        }
    }
}

/// MEAN: two-pass means of the complete cases.
fn mean_complete(x: &[f64], n: usize, nc: usize, ind: &[i32], nobs: usize) -> Vec<f64> {
    (0..nc)
        .map(|i| {
            let xx = col(x, n, i);
            let mut sum = ld(0.0);
            for k in 0..n {
                if ind[k] != 0 {
                    sum += xx[k];
                }
            }
            let mut tmp = sum / ld_int(nobs);
            if tmp.to_f64().is_finite() {
                sum = ld(0.0);
                for k in 0..n {
                    if ind[k] != 0 {
                        sum += dm(xx[k], tmp);
                    }
                }
                tmp = tmp + sum / ld_int(nobs);
            }
            tmp.to_f64()
        })
        .collect()
}

/// MEAN_: two-pass means of columns without NAs.
fn mean_na(x: &[f64], n: usize, nc: usize, has_na: &[bool]) -> Vec<f64> {
    (0..nc)
        .map(|i| {
            if has_na[i] {
                return NA_REAL;
            }
            let xx = col(x, n, i);
            let sum = ld(0.0).sum_from(xx);
            let mut tmp = sum / ld_int(n);
            if tmp.to_f64().is_finite() {
                let sum = tmp.sum_dev(xx);
                tmp = tmp + sum / ld_int(n);
            }
            tmp.to_f64()
        })
        .collect()
}

fn fill_na(o: &mut Out, ncx: usize, ny: usize) {
    for i in 0..ncx {
        for j in 0..ny {
            o.set(i, j, NA_REAL);
        }
    }
}

/// Kendall's sum over complete pairs.
fn kendall_sum(xx: &[f64], yy: &[f64], ind: Option<&[i32]>) -> LD {
    let n = xx.len();
    let ok = |k: usize| ind.is_none_or(|ind| ind[k] != 0);
    let mut sum = ld(0.0);
    for k in 0..n {
        if ok(k) {
            for m in 0..n {
                if ok(m) {
                    sum += sign(xx[k] - xx[m]) * sign(yy[k] - yy[m]);
                }
            }
        }
    }
    sum
}

fn cross_sum(xx: &[f64], xxm: f64, yy: &[f64], yym: f64, ind: Option<&[i32]>) -> LD {
    let (xxm, yym) = (ld(xxm), ld(yym));
    if ind.is_none() {
        return xxm.sum_cross_dev(xx, yym, yy);
    }
    let mut sum = ld(0.0);
    for k in 0..xx.len() {
        if ind.is_none_or(|ind| ind[k] != 0) {
            sum += dm(xx[k], xxm) * dm(yy[k], yym);
        }
    }
    sum
}

/// Correlations from a symmetric covariance matrix (cov_complete1/cov_na_1).
/// `xm` holds the column means (NA for columns with NAs), which R reuses
/// for the standard deviations.
fn cor_from_cov1(o: &mut Out, ncx: usize, has_na: Option<&[bool]>, mut xm: Vec<f64>) {
    let na = |i: usize| has_na.is_some_and(|h| h[i]);
    for i in 0..ncx {
        if !na(i) {
            xm[i] = o.get(i, i).sqrt();
        }
    }
    for i in 0..ncx {
        if !na(i) {
            for j in 0..i {
                if xm[i] == 0.0 || xm[j] == 0.0 {
                    o.sd_0 = true;
                    o.set(j, i, NA_REAL);
                    o.set(i, j, NA_REAL);
                } else {
                    let sum = ld(o.get(i, j) / (xm[i] * xm[j]));
                    let v = clamp_ld(sum).to_f64();
                    o.set(j, i, v);
                    o.set(i, j, v);
                }
            }
        }
        o.set(i, i, 1.0);
    }
}

fn cov_complete1(o: &mut Out, n: usize, ncx: usize, x: &[f64], ind: &[i32], cor: bool, kendall: bool) {
    let nobs = ind.iter().filter(|&&v| v != 0).count();
    if nobs <= 1 {
        return fill_na(o, ncx, ncx);
    }
    let mut xm = vec![0.0; ncx];
    let mut n1 = 0;
    if !kendall {
        xm = mean_complete(x, n, ncx, ind, nobs);
        n1 = nobs - 1;
    }
    for i in 0..ncx {
        let xx = col(x, n, i);
        for j in 0..=i {
            let yy = col(x, n, j);
            let v = if !kendall {
                (cross_sum(xx, xm[i], yy, xm[j], Some(ind)) / ld_int(n1)).to_f64()
            } else {
                kendall_sum(xx, yy, Some(ind)).to_f64()
            };
            o.set(j, i, v);
            o.set(i, j, v);
        }
    }
    if cor {
        cor_from_cov1(o, ncx, None, xm);
    }
}

fn cov_na_1(o: &mut Out, n: usize, ncx: usize, x: &[f64], has_na: &[bool], cor: bool, kendall: bool) {
    if n <= 1 {
        return fill_na(o, ncx, ncx);
    }
    let mut xm = vec![NA_REAL; ncx];
    let mut n1 = 0;
    if !kendall {
        xm = mean_na(x, n, ncx, has_na);
        n1 = n - 1;
    }
    for i in 0..ncx {
        if has_na[i] {
            for j in 0..=i {
                o.set(j, i, NA_REAL);
                o.set(i, j, NA_REAL);
            }
            continue;
        }
        let xx = col(x, n, i);
        for j in 0..=i {
            let v = if has_na[j] {
                NA_REAL
            } else if !kendall {
                (cross_sum(xx, xm[i], col(x, n, j), xm[j], None) / ld_int(n1)).to_f64()
            } else {
                kendall_sum(xx, col(x, n, j), None).to_f64()
            };
            o.set(j, i, v);
            o.set(i, j, v);
        }
    }
    if cor {
        cor_from_cov1(o, ncx, Some(has_na), xm);
    }
}

/// COV_SDEV: standard deviations of the columns (complete cases or
/// columns without NAs), replacing the means.
fn sdev(
    x: &[f64],
    n: usize,
    nc: usize,
    means: &mut [f64],
    ind: Option<&[i32]>,
    has_na: Option<&[bool]>,
    n1: usize,
    kendall: bool,
) {
    let ok = |k: usize| ind.is_none_or(|ind| ind[k] != 0);
    for i in 0..nc {
        if has_na.is_some_and(|h| h[i]) {
            continue;
        }
        let xx = col(x, n, i);
        let mut sum = ld(0.0);
        if !kendall {
            let xxm = ld(means[i]);
            if ind.is_none() {
                sum = xxm.sum_sq_dev(xx);
            } else {
                for k in 0..n {
                    if ok(k) {
                        sum += dm(xx[k], xxm) * dm(xx[k], xxm);
                    }
                }
            }
            sum /= ld_int(n1);
        } else {
            for k in 0..n {
                if ok(k) {
                    for m in 0..n {
                        if ok(m) && xx[k] != xx[m] {
                            sum += 1.0;
                        }
                    }
                }
            }
        }
        means[i] = sum.sqrt().to_f64();
    }
}

#[allow(clippy::too_many_arguments)]
fn cor_from_sd2(
    o: &mut Out,
    ncx: usize,
    ncy: usize,
    xm: &[f64],
    ym: &[f64],
    na_x: Option<&[bool]>,
    na_y: Option<&[bool]>,
) {
    for i in 0..ncx {
        if na_x.is_some_and(|h| h[i]) {
            continue;
        }
        for j in 0..ncy {
            if na_y.is_some_and(|h| h[j]) {
                continue;
            }
            if xm[i] == 0.0 || ym[j] == 0.0 {
                o.sd_0 = true;
                o.set(i, j, NA_REAL);
            } else {
                let v = o.get(i, j) / (xm[i] * ym[j]);
                o.set(i, j, clamp(v));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cov_complete2(
    o: &mut Out,
    n: usize,
    ncx: usize,
    ncy: usize,
    x: &[f64],
    y: &[f64],
    ind: &[i32],
    cor: bool,
    kendall: bool,
) {
    let nobs = ind.iter().filter(|&&v| v != 0).count();
    if nobs <= 1 {
        return fill_na(o, ncx, ncy);
    }
    let (mut xm, mut ym) = (vec![0.0; ncx], vec![0.0; ncy]);
    let mut n1 = 0;
    if !kendall {
        xm = mean_complete(x, n, ncx, ind, nobs);
        ym = mean_complete(y, n, ncy, ind, nobs);
        n1 = nobs - 1;
    }
    for i in 0..ncx {
        let xx = col(x, n, i);
        for j in 0..ncy {
            let yy = col(y, n, j);
            let v = if !kendall {
                (cross_sum(xx, xm[i], yy, ym[j], Some(ind)) / ld_int(n1)).to_f64()
            } else {
                kendall_sum(xx, yy, Some(ind)).to_f64()
            };
            o.set(i, j, v);
        }
    }
    if cor {
        sdev(x, n, ncx, &mut xm, Some(ind), None, n1, kendall);
        sdev(y, n, ncy, &mut ym, Some(ind), None, n1, kendall);
        cor_from_sd2(o, ncx, ncy, &xm, &ym, None, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn cov_na_2(
    o: &mut Out,
    n: usize,
    ncx: usize,
    ncy: usize,
    x: &[f64],
    y: &[f64],
    na_x: &[bool],
    na_y: &[bool],
    cor: bool,
    kendall: bool,
) {
    if n <= 1 {
        return fill_na(o, ncx, ncy);
    }
    let (mut xm, mut ym) = (vec![NA_REAL; ncx], vec![NA_REAL; ncy]);
    let mut n1 = 0;
    if !kendall {
        xm = mean_na(x, n, ncx, na_x);
        ym = mean_na(y, n, ncy, na_y);
        n1 = n - 1;
    }
    for i in 0..ncx {
        if na_x[i] {
            for j in 0..ncy {
                o.set(i, j, NA_REAL);
            }
            continue;
        }
        let xx = col(x, n, i);
        for j in 0..ncy {
            let v = if na_y[j] {
                NA_REAL
            } else if !kendall {
                (cross_sum(xx, xm[i], col(y, n, j), ym[j], None) / ld_int(n1)).to_f64()
            } else {
                kendall_sum(xx, col(y, n, j), None).to_f64()
            };
            o.set(i, j, v);
        }
    }
    if cor {
        sdev(x, n, ncx, &mut xm, None, Some(na_x), n1, kendall);
        sdev(y, n, ncy, &mut ym, None, Some(na_y), n1, kendall);
        cor_from_sd2(o, ncx, ncy, &xm, &ym, Some(na_x), Some(na_y));
    }
}

/// complete1/complete2: indicators of complete cases.
fn complete(n: usize, cols: &[(&[f64], usize)], na_fail: bool) -> RResult<Vec<i32>> {
    let mut ind = vec![1; n];
    for (x, nc) in cols {
        for j in 0..*nc {
            for (i, v) in col(x, n, j).iter().enumerate() {
                if v.is_nan() {
                    if na_fail {
                        return Err(Flow::error("missing observations in cov/cor"));
                    }
                    ind[i] = 0;
                }
            }
        }
    }
    Ok(ind)
}

fn find_na(x: &[f64], n: usize, nc: usize) -> Vec<bool> {
    (0..nc).map(|j| col(x, n, j).iter().any(|v| v.is_nan())).collect()
}

const VAR_FACTOR_MSG: &str = "Calling var(x) on a factor x is defunct.\n  Use something like 'all(duplicated(x)[-1L])' to test for a constant vector.";

/// The data and shape of `x`: (values, n, ncol, is a matrix).
fn as_real_data(i: &mut Interp, x: &Value) -> RResult<(Vec<f64>, Value)> {
    let v = i.coerce(x, VType::Real)?;
    let Value::Real(r) = &v else { return Err(Flow::error("'x' is NULL")) };
    Ok((r.data.clone(), v))
}

fn matrix_dims(v: &Value) -> Option<(usize, usize)> {
    match v.get_attr(s::DIM) {
        Some(Value::Int(d)) if d.data.len() == 2 => Some((d.data[0] as usize, d.data[1] as usize)),
        _ => None,
    }
}

fn col_names(v: &Value) -> Option<Value> {
    match v.get_attr(s::DIMNAMES) {
        Some(Value::List(l)) if l.data.len() >= 2 && !l.data[1].is_null() => Some(l.data[1].clone()),
        _ => None,
    }
}

// R: stats/src/cov.c:corcov
pub fn corcov(i: &mut Interp, x: &Value, y: &Value, na_method: &Value, skendall: &Value, cor: bool) -> RResult<Value> {
    if x.is_null() {
        return Err(Flow::error("'x' is NULL"));
    }
    if x.inherits("factor") {
        return Err(Flow::error(VAR_FACTOR_MSG));
    }
    let (xd, xv) = as_real_data(i, x)?;
    let mut ansmat = matrix_dims(&xv).is_some();
    let (n, ncx) = matrix_dims(&xv).unwrap_or((xd.len(), 1));
    let mut yd = Vec::new();
    let mut yv = Value::Null;
    let ncy = if y.is_null() {
        ncx
    } else {
        if y.inherits("factor") {
            return Err(Flow::error(VAR_FACTOR_MSG));
        }
        let (d, v) = as_real_data(i, y)?;
        let ncy = match matrix_dims(&v) {
            Some((nr, nc)) => {
                if nr != n {
                    return Err(Flow::error("incompatible dimensions"));
                }
                ansmat = true;
                nc
            }
            None => {
                if d.len() != n {
                    return Err(Flow::error("incompatible dimensions"));
                }
                1
            }
        };
        yd = d;
        yv = v;
        ncy
    };
    let method = crate::coerce::as_integer(na_method);
    let kendall = crate::coerce::as_logical(skendall);
    if kendall == NA_LOGICAL {
        return Err(Flow::error("invalid 'kendall' argument"));
    }
    let kendall = kendall != 0;
    let (mut na_fail, mut everything, mut empty_err, mut pair) = (false, false, true, false);
    match method {
        1 => na_fail = true,
        2 => {
            if xd.is_empty() {
                return Err(Flow::error("no complete element pairs"));
            }
        }
        3 => pair = true,
        4 => {
            everything = true;
            empty_err = false;
        }
        5 => empty_err = false,
        _ => return Err(Flow::error("invalid 'use' (computational method)")),
    }
    if empty_err && xd.is_empty() {
        return Err(Flow::error("'x' is empty"));
    }
    let mut o = Out { ans: vec![0.0; ncx * ncy], ncx, sd_0: false };
    if y.is_null() {
        if everything {
            let has_na = find_na(&xd, n, ncx);
            cov_na_1(&mut o, n, ncx, &xd, &has_na, cor, kendall);
        } else if !pair {
            let ind = complete(n, &[(&xd, ncx)], na_fail)?;
            cov_complete1(&mut o, n, ncx, &xd, &ind, cor, kendall);
            if empty_err && !ind.contains(&1) {
                return Err(Flow::error("no complete element pairs"));
            }
        } else {
            cov_pairwise1(&mut o, n, ncx, &xd, cor, kendall);
        }
    } else if everything {
        let na_x = find_na(&xd, n, ncx);
        let na_y = find_na(&yd, n, ncy);
        cov_na_2(&mut o, n, ncx, ncy, &xd, &yd, &na_x, &na_y, cor, kendall);
    } else if !pair {
        let ind = complete(n, &[(&xd, ncx), (&yd, ncy)], na_fail)?;
        cov_complete2(&mut o, n, ncx, ncy, &xd, &yd, &ind, cor, kendall);
        if empty_err && !ind.contains(&1) {
            return Err(Flow::error("no complete element pairs"));
        }
    } else {
        cov_pairwise2(&mut o, n, ncx, ncy, &xd, &yd, cor, kendall);
    }
    let mut ans = Value::reals(o.ans);
    if ansmat {
        ans.set_attr_raw(s::DIM, Value::ints(vec![ncx as i32, ncy as i32]));
        let (xn, yn) = if y.is_null() {
            let c = col_names(&xv);
            (c.clone(), c)
        } else {
            (col_names(&xv), col_names(&yv))
        };
        if xn.is_some() || yn.is_some() {
            ans.set_attr_raw(s::DIMNAMES, Value::list(vec![xn.unwrap_or(Value::Null), yn.unwrap_or(Value::Null)]));
        }
    }
    if o.sd_0 {
        i.warning("the standard deviation is zero")?;
    }
    Ok(ans)
}
