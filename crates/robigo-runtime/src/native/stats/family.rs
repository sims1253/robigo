//! The binomial family's link and deviance (port of GNU R's
//! src/library/stats/src/family.c).

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::value::*;

const THRESH: f64 = 30.0;
const MTHRESH: f64 = -30.0;
const INVEPS: f64 = 1.0 / f64::EPSILON;

fn is_numeric(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Lgl(_) | Value::Real(_)) && !v.inherits("factor")
}

/// `shallow_duplicate(x)` with new contents.
fn like(x: &Value, data: Vec<f64>) -> Value {
    let mut v = Value::reals(data);
    v.set_attrs(x.attrs().cloned().map(Box::new));
    v
}

fn x_d_omx(x: f64) -> RResult<f64> {
    if !(0.0..=1.0).contains(&x) {
        return Err(Flow::error(format!("Value {} out of range (0, 1)", robigo_syntax::cfmt::fmt_g(x, 6, false, false))));
    }
    Ok(x / (1.0 - x))
}

// R: family.c:logit_link
pub fn logit_link(_: &mut Interp, mu: &Value) -> RResult<Value> {
    let Value::Real(m) = mu else { return Err(Flow::error("Argument mu must be a nonempty numeric vector")) };
    if m.data.is_empty() {
        return Err(Flow::error("Argument mu must be a nonempty numeric vector"));
    }
    let out = m.data.iter().map(|&x| x_d_omx(x).map(f64::ln)).collect::<RResult<Vec<f64>>>()?;
    Ok(like(mu, out))
}

fn reals_of(i: &mut Interp, v: &Value, what: &str) -> RResult<(Value, Vec<f64>)> {
    if v.is_empty() || !is_numeric(v) {
        return Err(Flow::error(format!("Argument {} must be a nonempty numeric vector", what)));
    }
    let r = i.coerce(v, VType::Real)?;
    let Value::Real(d) = &r else { unreachable!() };
    let d = d.data.clone();
    Ok((r, d))
}

// R: family.c:logit_linkinv
pub fn logit_linkinv(i: &mut Interp, eta: &Value) -> RResult<Value> {
    let (r, e) = reals_of(i, eta, "eta")?;
    let out = e
        .iter()
        .map(|&etai| {
            let tmp = if etai < MTHRESH {
                f64::EPSILON
            } else if etai > THRESH {
                INVEPS
            } else {
                etai.exp()
            };
            tmp / (1.0 + tmp)
        })
        .collect();
    Ok(like(&r, out))
}

// R: family.c:logit_mu_eta
pub fn logit_mu_eta(i: &mut Interp, eta: &Value) -> RResult<Value> {
    let (r, e) = reals_of(i, eta, "eta")?;
    let out = e
        .iter()
        .map(|&etai| {
            let exp_e = etai.exp();
            let opexp = 1.0 + exp_e;
            if etai > THRESH || etai < MTHRESH { f64::EPSILON } else { exp_e / (opexp * opexp) }
        })
        .collect();
    Ok(like(&r, out))
}

fn y_log_y(y: f64, mu: f64) -> f64 {
    if y != 0.0 { y * (y / mu).ln() } else { 0.0 }
}

// R: family.c:binomial_dev_resids
pub fn binomial_dev_resids(i: &mut Interp, y: &Value, mu: &Value, wt: &Value) -> RResult<Value> {
    let yr = i.coerce(y, VType::Real)?;
    let Value::Real(ry) = &yr else { unreachable!() };
    let n = ry.data.len();
    let (lmu, lwt) = (mu.len(), wt.len());
    let rmu = crate::coerce::to_real(&i.coerce(mu, VType::Real)?, &mut Default::default());
    let rwt = crate::coerce::to_real(&i.coerce(wt, VType::Real)?, &mut Default::default());
    if lmu != n && lmu != 1 {
        return Err(Flow::error(format!("argument mu must be a numeric vector of length 1 or length {}", n)));
    }
    if lwt != n && lwt != 1 {
        return Err(Flow::error(format!("argument wt must be a numeric vector of length 1 or length {}", n)));
    }
    let out = (0..n)
        .map(|k| {
            let mui = if lmu > 1 { rmu[k] } else { rmu[0] };
            let yi = ry.data[k];
            2.0 * rwt[if lwt > 1 { k } else { 0 }] * (y_log_y(yi, mui) + y_log_y(1.0 - yi, 1.0 - mui))
        })
        .collect();
    Ok(like(&yr, out))
}
