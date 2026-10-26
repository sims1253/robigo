//! `C_Cdqrls`: least squares by QR for `lm.fit` (port of GNU R's
//! src/library/stats/src/lm.c).

use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::native::linpack;
use crate::sym::s;
use crate::value::*;

// R: stats/src/lm.c:Cdqrls
pub fn cdqrls(i: &mut Interp, x: &Value, y: &Value, tol: &Value, chk: &Value) -> RResult<Value> {
    let rtol = crate::coerce::as_real(tol);
    let check = crate::coerce::as_logical(chk) == TRUE;
    let dims = crate::attrib::dims(x).unwrap_or_default();
    if check && dims.len() != 2 {
        return Err(Flow::error("'x' is not a matrix"));
    }
    let (n, p) = (dims[0] as usize, dims[1] as usize);
    let ylen = y.len();
    let ny = if n > 0 { ylen / n } else { 0 };
    if check && n * ny != ylen {
        return Err(Flow::error(format!("dimensions of 'x' ({},{}) and 'y' ({}) do not match", n, p, ylen)));
    }
    let xr = i.coerce(x, VType::Real)?;
    let yr = i.coerce(y, VType::Real)?;
    let Value::Real(xv) = &xr else { unreachable!() };
    let Value::Real(yv) = &yr else { unreachable!() };
    if xv.data.iter().any(|v| !v.is_finite()) {
        return Err(Flow::error("NA/NaN/Inf in 'x'"));
    }
    if yv.data.iter().any(|v| !v.is_finite()) {
        return Err(Flow::error("NA/NaN/Inf in 'y'"));
    }
    let mut qr = xv.data.clone();
    let mut coef = vec![0.0; p * ny];
    let mut rsd = yv.data.clone();
    let mut effects = yv.data.clone();
    let mut pivot: Vec<i32> = (1..=p as i32).collect();
    let mut qraux = vec![0.0; p];
    let mut work = vec![0.0; 2 * p];
    let rank = linpack::dqrls(&mut qr, n, p, &yv.data, ny, rtol, &mut coef, &mut rsd, &mut effects, &mut pivot, &mut qraux, &mut work);
    let pivoted = pivot.iter().enumerate().any(|(k, &v)| v != k as i32 + 1);
    // qr, residuals and effects keep the attributes of x and y.
    let with_attrs = |data: Vec<f64>, from: &Value| {
        let mut v = Value::reals(data);
        v.set_attrs(from.attrs().cloned().map(Box::new));
        v
    };
    let mut coefficients = Value::reals(coef);
    if ny > 1 {
        coefficients.set_attr_raw(s::DIM, Value::ints(vec![p as i32, ny as i32]));
    }
    Ok(Value::named_list(vec![
        ("qr", with_attrs(qr, &xr)),
        ("coefficients", coefficients),
        ("residuals", with_attrs(rsd, &yr)),
        ("effects", with_attrs(effects, &yr)),
        ("rank", Value::int(rank as i32)),
        ("pivot", Value::ints(pivot)),
        ("qraux", Value::reals(qraux)),
        ("tol", tol.clone()),
        ("pivoted", Value::lgl(pivoted)),
    ]))
}
