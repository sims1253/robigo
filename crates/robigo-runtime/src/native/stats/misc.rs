//! Smaller stats routines: interpolation (approx.c), complete cases
//! (complete_cases.c), distances (distance.c), linear filters and
//! autocorrelation (filter.c), and Brent's one-dimensional minimizer and
//! root finder (optimize.c, zeroin.c).

use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

fn reals(v: &Value) -> Vec<f64> {
    match v {
        Value::Real(r) => r.data.clone(),
        v => crate::coerce::to_real(v, &mut Default::default()),
    }
}

fn arg(a: &[Arg], k: usize) -> Value {
    a.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null)
}

fn as_int(v: &Value) -> i32 {
    crate::coerce::as_integer(v)
}

fn as_real(v: &Value) -> f64 {
    crate::coerce::as_real(v)
}

fn as_lgl(v: &Value) -> Lgl {
    crate::coerce::as_logical(v)
}

// ----------------------------------------------------------------------
// approx.c

// R: approx.c:R_approxtest
pub fn approx_test(a: &[Arg]) -> RResult<Value> {
    let (x, y) = (reals(&arg(a, 0)), reals(&arg(a, 1)));
    let method = as_int(&arg(a, 2));
    let f = as_real(&arg(a, 3));
    let na_rm = as_lgl(&arg(a, 4));
    match method {
        1 => {}
        2 => {
            if !f.is_finite() || !(0.0..=1.0).contains(&f) {
                return Err(Flow::error("approx(): invalid f value"));
            }
        }
        _ => return Err(Flow::error("approx(): invalid interpolation method")),
    }
    if na_rm != 0 {
        if x.iter().zip(y.iter()).any(|(a, b)| a.is_nan() || b.is_nan()) {
            return Err(Flow::error("approx(): attempted to interpolate NA values"));
        }
    } else if x.iter().any(|a| a.is_nan()) {
        return Err(Flow::error("approx(x,y, .., na.rm=FALSE): NA values in x are not allowed"));
    }
    Ok(Value::Null)
}

// R: approx.c:Approx, R_approxfun, approx1
pub fn approx(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let (x, y) = (reals(&arg(a, 0)), reals(&arg(a, 1)));
    let xout = reals(&i.coerce(&arg(a, 2), VType::Real)?);
    let kind = as_int(&arg(a, 3));
    let (ylow, yhigh, f) = (as_real(&arg(a, 4)), as_real(&arg(a, 5)), as_real(&arg(a, 6)));
    let (f1, f2) = (1.0 - f, f);
    let n = x.len();
    let approx1 = |v: f64| -> f64 {
        if n == 0 {
            return f64::NAN;
        }
        let (mut i, mut j) = (0usize, n - 1);
        if v < x[i] {
            return ylow;
        }
        if v > x[j] {
            return yhigh;
        }
        while i + 1 < j {
            let ij = (i + j) / 2;
            if v < x[ij] {
                j = ij;
            } else {
                i = ij;
            }
        }
        if v == x[j] {
            return y[j];
        }
        if v == x[i] {
            return y[i];
        }
        if kind == 1 {
            y[i] + (y[j] - y[i]) * ((v - x[i]) / (x[j] - x[i]))
        } else {
            (if f1 != 0.0 { y[i] * f1 } else { 0.0 }) + (if f2 != 0.0 { y[j] * f2 } else { 0.0 })
        }
    };
    Ok(Value::reals(xout.iter().map(|&v| if v.is_nan() { v } else { approx1(v) }).collect()))
}

// ----------------------------------------------------------------------
// complete_cases.c

fn type_err(v: &Value) -> Flow {
    Flow::error(format!("invalid 'type' ({}) of argument", v.type_name()))
}

fn rows_of(v: &Value) -> Option<usize> {
    match crate::attrib::dims(v) {
        Some(d) if d.len() == 2 => Some(d[0] as usize),
        _ => None,
    }
}

fn is_vector(v: &Value) -> bool {
    v.is_vector()
}

/// Mark the cases with NAs in `u` (recycled over `len`).
fn mark_na(u: &Value, len: usize, rval: &mut [i32]) -> RResult<()> {
    match u {
        Value::Int(x) | Value::Lgl(x) => {
            for (k, v) in x.data.iter().enumerate() {
                if *v == NA_INTEGER {
                    rval[k % len] = 0;
                }
            }
        }
        Value::Real(x) => {
            for (k, v) in x.data.iter().enumerate() {
                if v.is_nan() {
                    rval[k % len] = 0;
                }
            }
        }
        Value::Cplx(x) => {
            for (k, v) in x.data.iter().enumerate() {
                if v.re.is_nan() || v.im.is_nan() {
                    rval[k % len] = 0;
                }
            }
        }
        Value::Str(x) => {
            for (k, v) in x.data.iter().enumerate() {
                if v.is_none() {
                    rval[k % len] = 0;
                }
            }
        }
        Value::Null => {}
        v => return Err(type_err(v)),
    }
    Ok(())
}

// R: complete_cases.c:compcases
pub fn compcases(a: &[Arg]) -> RResult<Value> {
    let bad = || Flow::error("not all arguments have the same length");
    let mut len: i64 = -1;
    let check = |n: usize, len: &mut i64| -> RResult<()> {
        if *len < 0 {
            *len = n as i64;
            Ok(())
        } else if *len != n as i64 {
            Err(bad())
        } else {
            Ok(())
        }
    };
    for s in a {
        let v = &s.value;
        match v {
            Value::Null => {}
            Value::Pairlist(p) => {
                for t in &p.items {
                    if let Some(r) = rows_of(&t.value) {
                        check(r, &mut len)?;
                    } else if is_vector(&t.value) {
                        check(t.value.len(), &mut len)?;
                    } else {
                        return Err(type_err(&t.value));
                    }
                }
            }
            Value::List(l) => {
                if !l.data.is_empty() {
                    for t in &l.data {
                        if let Some(r) = rows_of(t) {
                            check(r, &mut len)?;
                        } else if is_vector(t) {
                            check(t.len(), &mut len)?;
                        } else {
                            return Err(Flow::error("invalid 'type' (unknown) of argument"));
                        }
                    }
                } else {
                    let u = crate::attrib::get_attrib(v, Sym::new("row.names"));
                    if !u.is_null() {
                        if len < 0 {
                            len = u.len() as i64;
                        } else if Some(len as i32) != crate::coerce::to_int(&u, &mut Default::default()).first().copied() {
                            return Err(bad());
                        }
                    }
                }
            }
            v => {
                if let Some(r) = rows_of(v) {
                    check(r, &mut len)?;
                } else if is_vector(v) {
                    check(v.len(), &mut len)?;
                } else {
                    return Err(type_err(v));
                }
            }
        }
    }
    if len < 0 {
        return Err(Flow::error("no input has determined the number of cases"));
    }
    let len = len as usize;
    let mut rval = vec![1i32; len];
    for s in a {
        match &s.value {
            Value::Pairlist(p) => {
                for t in &p.items {
                    mark_na(&t.value, len, &mut rval)?;
                }
                // R falls through to the vector case, which rejects pairlists.
                if !p.items.is_empty() {
                    return Err(type_err(&s.value));
                }
            }
            Value::List(l) => {
                for t in &l.data {
                    mark_na(t, len, &mut rval)?;
                }
            }
            v => mark_na(v, len, &mut rval)?,
        }
    }
    Ok(Value::lgls(rval))
}

// ----------------------------------------------------------------------
// distance.c

// R: distance.c:Cdist, R_distance
pub fn cdist(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let xv = arg(a, 0);
    let d = crate::attrib::dims(&xv).unwrap_or_default();
    let (nr, nc) = if d.len() == 2 { (d[0] as usize, d[1] as usize) } else { (xv.len(), 1) };
    let method = as_int(&arg(a, 1));
    let attrs = arg(a, 2);
    let p = as_real(&arg(a, 3));
    let x = reals(&i.coerce(&xv, VType::Real)?);
    if !(1..=6).contains(&method) {
        return Err(Flow::error("distance(): invalid distance"));
    }
    if method == 6 && (!p.is_finite() || p <= 0.0) {
        return Err(Flow::error("distance(): invalid p"));
    }
    let non_na = |a: f64, b: f64| !a.is_nan() && !b.is_nan();
    let mut warn_nonfinite = 0;
    let mut dist = |i1: usize, i2: usize| -> f64 {
        let col = |j: usize| (x[i1 + j * nr], x[i2 + j * nr]);
        match method {
            1 | 3 | 6 => {
                let (mut dist, mut count) = (0.0, 0usize);
                for j in 0..nc {
                    let (a, b) = col(j);
                    if non_na(a, b) {
                        let dev = a - b;
                        if !dev.is_nan() {
                            dist += match method {
                                1 => dev * dev,
                                3 => dev.abs(),
                                _ => crate::builtins::arith::r_pow(dev.abs(), p),
                            };
                            count += 1;
                        }
                    }
                }
                if count == 0 {
                    return NA_REAL;
                }
                if count != nc {
                    dist /= count as f64 / nc as f64;
                }
                match method {
                    1 => dist.sqrt(),
                    3 => dist,
                    _ => crate::builtins::arith::r_pow(dist, 1.0 / p),
                }
            }
            2 => {
                let (mut dist, mut count) = (-f64::MAX, 0);
                for j in 0..nc {
                    let (a, b) = col(j);
                    if non_na(a, b) {
                        let dev = (a - b).abs();
                        if !dev.is_nan() {
                            if dev > dist {
                                dist = dev;
                            }
                            count += 1;
                        }
                    }
                }
                if count == 0 { NA_REAL } else { dist }
            }
            4 => {
                let (mut dist, mut count) = (0.0, 0usize);
                for j in 0..nc {
                    let (a, b) = col(j);
                    if non_na(a, b) {
                        let sum = a.abs() + b.abs();
                        let diff = (a - b).abs();
                        if sum > f64::MIN_POSITIVE || diff > f64::MIN_POSITIVE {
                            let mut dev = diff / sum;
                            let ok = if !dev.is_nan() {
                                true
                            } else if !diff.is_finite() && diff == sum {
                                dev = 1.0;
                                true
                            } else {
                                false
                            };
                            if ok {
                                dist += dev;
                                count += 1;
                            }
                        }
                    }
                }
                if count == 0 {
                    return NA_REAL;
                }
                if count != nc {
                    dist /= count as f64 / nc as f64;
                }
                dist
            }
            _ => {
                let (mut total, mut count, mut dist) = (0, 0, 0);
                for j in 0..nc {
                    let (a, b) = col(j);
                    if non_na(a, b) {
                        if !(a.is_finite() && b.is_finite()) {
                            warn_nonfinite += 1;
                        } else {
                            if a != 0.0 || b != 0.0 {
                                count += 1;
                                if !(a != 0.0 && b != 0.0) {
                                    dist += 1;
                                }
                            }
                            total += 1;
                        }
                    }
                }
                if total == 0 {
                    NA_REAL
                } else if count == 0 {
                    0.0
                } else {
                    dist as f64 / count as f64
                }
            }
        }
    };
    let mut d = Vec::with_capacity(nr * nr.saturating_sub(1) / 2);
    for j in 0..=nr {
        for i2 in (j + 1)..nr {
            d.push(dist(i2, j));
        }
    }
    for _ in 0..warn_nonfinite {
        i.warning("treating non-finite values as NA")?;
    }
    let mut ans = Value::reals(d);
    if let (Value::List(l), Some(Value::Str(names))) = (&attrs, attrs.names()) {
        for (v, n) in l.data.iter().zip(names.data.iter()) {
            let name = n.as_deref().unwrap_or("NA");
            ans = crate::attrib::set_attrib(ans, Sym::new(name), v.clone())?;
        }
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// filter.c

fn isok(x: f64) -> bool {
    !x.is_nan()
}

// R: filter.c:cfilter
pub fn cfilter(a: &[Arg]) -> RResult<Value> {
    let (Value::Real(xs), Value::Real(fs)) = (arg(a, 0), arg(a, 1)) else {
        return Err(Flow::error("invalid input"));
    };
    let (x, filter) = (&xs.data, &fs.data);
    let (nx, nf) = (x.len() as i64, filter.len() as i64);
    let sides = as_int(&arg(a, 2));
    let circular = as_lgl(&arg(a, 3));
    if sides == NA_INTEGER || circular == NA_LOGICAL {
        return Err(Flow::error("invalid input"));
    }
    let nshift = if sides == 2 { nf / 2 } else { 0 };
    let mut out = vec![0.0; nx as usize];
    for i in 0..nx {
        let mut z = 0.0;
        if circular == 0 {
            if i + nshift - (nf - 1) < 0 || i + nshift >= nx {
                out[i as usize] = NA_REAL;
                continue;
            }
            let mut na = false;
            for j in (nshift + i - nx).max(0)..nf.min(i + nshift + 1) {
                let tmp = x[(i + nshift - j) as usize];
                if isok(tmp) {
                    z += filter[j as usize] * tmp;
                } else {
                    na = true;
                    break;
                }
            }
            out[i as usize] = if na { NA_REAL } else { z };
        } else {
            let mut na = false;
            for j in 0..nf {
                let mut ii = i + nshift - j;
                if ii < 0 {
                    ii += nx;
                }
                if ii >= nx {
                    ii -= nx;
                }
                let tmp = x[ii as usize];
                if isok(tmp) {
                    z += filter[j as usize] * tmp;
                } else {
                    na = true;
                    break;
                }
            }
            out[i as usize] = if na { NA_REAL } else { z };
        }
    }
    Ok(Value::reals(out))
}

// R: filter.c:rfilter
pub fn rfilter(a: &[Arg]) -> RResult<Value> {
    let (Value::Real(xs), Value::Real(fs), Value::Real(os)) = (arg(a, 0), arg(a, 1), arg(a, 2)) else {
        return Err(Flow::error("invalid input"));
    };
    let (rx, rf) = (&xs.data, &fs.data);
    let mut r = os.data.clone();
    let nf = rf.len();
    for i in 0..rx.len() {
        let mut sum = rx[i];
        if !isok(sum) {
            r[nf + i] = NA_REAL;
            continue;
        }
        let mut na = false;
        for j in 0..nf {
            let tmp = r[nf + i - j - 1];
            if isok(tmp) {
                sum += tmp * rf[j];
            } else {
                na = true;
                break;
            }
        }
        r[nf + i] = if na { NA_REAL } else { sum };
    }
    let mut v = Value::reals(r);
    v.set_attrs(arg(a, 2).attrs().cloned().map(Box::new));
    Ok(v)
}

// R: filter.c:acf, acf0
pub fn acf(i: &mut Interp, a: &[Arg]) -> RResult<Value> {
    let xv = arg(a, 0);
    let d = crate::attrib::dims(&xv).unwrap_or_default();
    let (n, ns) = if d.len() == 2 { (d[0] as usize, d[1] as usize) } else { (xv.len(), 1) };
    let nl = as_int(&arg(a, 1)).max(0) as usize;
    let cor = as_lgl(&arg(a, 2)) == TRUE;
    let x = reals(&i.coerce(&xv, VType::Real)?);
    let (d1, d2) = (nl + 1, ns * (nl + 1));
    let mut acf = vec![0.0; (nl + 1) * ns * ns];
    for u in 0..ns {
        for v in 0..ns {
            for lag in 0..=nl {
                let (mut sum, mut nu) = (0.0, 0usize);
                for k in 0..n.saturating_sub(lag) {
                    let (p, q) = (x[k + lag + n * u], x[k + n * v]);
                    if !p.is_nan() && !q.is_nan() {
                        nu += 1;
                        sum += p * q;
                    }
                }
                acf[lag + d1 * u + d2 * v] = if nu > 0 { sum / (nu + lag) as f64 } else { NA_REAL };
            }
        }
    }
    if cor {
        if n == 1 {
            for u in 0..ns {
                acf[d1 * u + d2 * u] = 1.0;
            }
        } else {
            let se: Vec<f64> = (0..ns).map(|u| acf[d1 * u + d2 * u].sqrt()).collect();
            for u in 0..ns {
                for v in 0..ns {
                    for lag in 0..=nl {
                        let a = acf[lag + d1 * u + d2 * v] / (se[u] * se[v]);
                        acf[lag + d1 * u + d2 * v] = if a > 1.0 { 1.0 } else if a < -1.0 { -1.0 } else { a };
                    }
                }
            }
        }
    }
    let mut ans = Value::reals(acf);
    ans.set_attr_raw(s::DIM, Value::ints(vec![nl as i32 + 1, ns as i32, ns as i32]));
    Ok(ans)
}

// ----------------------------------------------------------------------
// optimize.c, zeroin.c

/// Evaluate `f(x)` for the minimizer or root finder.
// R: optimize.c:fcn1, fcn2
fn call_fn(i: &mut Interp, f: &Value, x: f64, env: &Env, what: &str) -> RResult<f64> {
    let call = Value::lang(f.clone(), vec![Arg::pos(Value::real(x))]);
    let s = i.eval(&call, env)?;
    match &s {
        Value::Int(v) if v.data.len() == 1 => {
            if v.data[0] == NA_INTEGER {
                i.warning("NA replaced by maximum positive value")?;
                Ok(f64::MAX)
            } else {
                Ok(v.data[0] as f64)
            }
        }
        Value::Real(v) if v.data.len() == 1 => {
            let r = v.data[0];
            if !r.is_finite() {
                if r == f64::NEG_INFINITY {
                    i.warning("-Inf replaced by maximally negative value")?;
                    Ok(-f64::MAX)
                } else {
                    i.warning(format!(
                        "{} replaced by maximum positive value",
                        if r.is_nan() { "NA/NaN" } else { "Inf" }
                    ))?;
                    Ok(f64::MAX)
                }
            } else {
                Ok(r)
            }
        }
        _ => Err(Flow::error(format!("invalid function value in '{}'", what))),
    }
}

fn check_fn(v: &Value) -> RResult<()> {
    if matches!(v, Value::Closure(_) | Value::Builtin(_)) {
        Ok(())
    } else {
        Err(Flow::error("attempt to minimize non-function"))
    }
}

fn finite_arg(v: &Value, what: &str) -> RResult<f64> {
    let x = as_real(v);
    if !x.is_finite() {
        return Err(Flow::error(format!("invalid '{}' value", what)));
    }
    Ok(x)
}

// R: optimize.c:do_fmin, Brent_fmin
pub fn do_fmin(i: &mut Interp, a: &[Arg], env: &Env) -> RResult<Value> {
    let f = arg(a, 0);
    check_fn(&f)?;
    let ax = finite_arg(&arg(a, 1), "xmin")?;
    let bx = finite_arg(&arg(a, 2), "xmax")?;
    if ax >= bx {
        return Err(Flow::error("'xmin' not less than 'xmax'"));
    }
    let tol = as_real(&arg(a, 3));
    if !tol.is_finite() || tol <= 0.0 {
        return Err(Flow::error("invalid 'tol' value"));
    }
    let c = (3.0 - 5f64.sqrt()) * 0.5;
    let eps = f64::EPSILON.sqrt();
    let (mut a_, mut b) = (ax, bx);
    let mut v = a_ + c * (b - a_);
    let (mut w, mut x) = (v, v);
    let (mut d, mut e) = (0.0f64, 0.0f64);
    let mut fx = call_fn(i, &f, x, env, "optimize")?;
    let (mut fv, mut fw) = (fx, fx);
    let tol3 = tol / 3.0;
    loop {
        let xm = (a_ + b) * 0.5;
        let tol1 = eps * x.abs() + tol3;
        let t2 = tol1 * 2.0;
        if (x - xm).abs() <= t2 - (b - a_) * 0.5 {
            break;
        }
        let (mut p, mut q, mut r) = (0.0, 0.0, 0.0);
        if e.abs() > tol1 {
            r = (x - w) * (fx - fv);
            q = (x - v) * (fx - fw);
            p = (x - v) * q - (x - w) * r;
            q = (q - r) * 2.0;
            if q > 0.0 {
                p = -p;
            } else {
                q = -q;
            }
            r = e;
            e = d;
        }
        if p.abs() >= (q * 0.5 * r).abs() || p <= q * (a_ - x) || p >= q * (b - x) {
            e = if x < xm { b - x } else { a_ - x };
            d = c * e;
        } else {
            d = p / q;
            let u = x + d;
            if u - a_ < t2 || b - u < t2 {
                d = tol1;
                if x >= xm {
                    d = -d;
                }
            }
        }
        let u = if d.abs() >= tol1 {
            x + d
        } else if d > 0.0 {
            x + tol1
        } else {
            x - tol1
        };
        let fu = call_fn(i, &f, u, env, "optimize")?;
        if fu <= fx {
            if u < x {
                b = x;
            } else {
                a_ = x;
            }
            v = w;
            w = x;
            x = u;
            fv = fw;
            fw = fx;
            fx = fu;
        } else {
            if u < x {
                a_ = u;
            } else {
                b = u;
            }
            if fu <= fw || w == x {
                v = w;
                fv = fw;
                w = u;
                fw = fu;
            } else if fu <= fv || v == x || v == w {
                v = u;
                fv = fu;
            }
        }
    }
    Ok(Value::real(x))
}

// R: optimize.c:zeroin2, zeroin.c:R_zeroin2
pub fn zeroin2(i: &mut Interp, a: &[Arg], env: &Env) -> RResult<Value> {
    let f = arg(a, 0);
    check_fn(&f)?;
    let ax = finite_arg(&arg(a, 1), "xmin")?;
    let bx = finite_arg(&arg(a, 2), "xmax")?;
    if ax >= bx {
        return Err(Flow::error("'xmin' not less than 'xmax'"));
    }
    let fa0 = as_real(&arg(a, 3));
    if fa0.is_nan() && fa0.to_bits() & 0xffff_ffff == 1954 {
        return Err(Flow::error("NA value for 'f.lower' is not allowed"));
    }
    let fb0 = as_real(&arg(a, 4));
    if fb0.is_nan() && fb0.to_bits() & 0xffff_ffff == 1954 {
        return Err(Flow::error("NA value for 'f.upper' is not allowed"));
    }
    let tol0 = as_real(&arg(a, 5));
    if !tol0.is_finite() || tol0 <= 0.0 {
        return Err(Flow::error("invalid 'tol' value"));
    }
    let maxit0 = as_int(&arg(a, 6));
    if maxit0 <= 0 {
        return Err(Flow::error("'maxiter' must be positive"));
    }
    let (mut a_, mut b) = (ax, bx);
    let (mut fa, mut fb) = (fa0, fb0);
    let mut c = a_;
    let mut fc = fa;
    let tol = tol0;
    let result = |root: f64, iter: f64, tol: f64| Value::reals(vec![root, iter, tol]);
    if fa == 0.0 {
        return Ok(result(a_, 0.0, 0.0));
    }
    if fb == 0.0 {
        return Ok(result(b, 0.0, 0.0));
    }
    let mut maxit = maxit0 + 1;
    while maxit > 0 {
        maxit -= 1;
        let prev_step = b - a_;
        if fc.abs() < fb.abs() {
            a_ = b;
            b = c;
            c = a_;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol_act = 2.0 * f64::EPSILON * b.abs() + tol / 2.0;
        let mut new_step = (c - b) / 2.0;
        if new_step.abs() <= tol_act || fb == 0.0 {
            return Ok(result(b, (maxit0 - maxit) as f64, (c - b).abs()));
        }
        if prev_step.abs() >= tol_act && fa.abs() > fb.abs() {
            let cb = c - b;
            let (mut p, mut q);
            if a_ == c {
                let t1 = fb / fa;
                p = cb * t1;
                q = 1.0 - t1;
            } else {
                q = fa / fc;
                let t1 = fb / fc;
                let t2 = fb / fa;
                p = t2 * (cb * q * (q - t1) - (b - a_) * (t1 - 1.0));
                q = (q - 1.0) * (t1 - 1.0) * (t2 - 1.0);
            }
            if p > 0.0 {
                q = -q;
            } else {
                p = -p;
            }
            if p < (0.75 * cb * q - (tol_act * q).abs() / 2.0) && p < (prev_step * q / 2.0).abs() {
                new_step = p / q;
            }
        }
        if new_step.abs() < tol_act {
            new_step = if new_step > 0.0 { tol_act } else { -tol_act };
        }
        a_ = b;
        fa = fb;
        b += new_step;
        fb = call_fn(i, &f, b, env, "zeroin")?;
        if (fb > 0.0 && fc > 0.0) || (fb < 0.0 && fc < 0.0) {
            c = a_;
            fc = fa;
        }
    }
    Ok(result(b, -1.0, (c - b).abs()))
}
