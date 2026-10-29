//! Arithmetic, comparison and logical operators and the Math group
//! (GNU R's arithmetic.c, relop.c, logic.c).

use std::rc::Rc;

use super::{arg, prim, special};
use crate::attrib::{copy_most_attrib, get_attrib, set_attrib};
use crate::coerce::{CoerceWarnings, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

use super::arith_exact::{add_mul, exact, x_is_dest};

pub static DEFS: &[Builtin] = &[
    prim("+", do_plus, -1),
    prim("-", do_minus, -1),
    prim("*", do_times, 2),
    prim("/", do_div, 2),
    prim("^", do_pow, 2),
    prim("%%", do_mod, 2),
    prim("%/%", do_idiv, 2),
    prim("==", do_eq, 2),
    prim("!=", do_ne, 2),
    prim("<", do_lt, 2),
    prim("<=", do_le, 2),
    prim(">", do_gt, 2),
    prim(">=", do_ge, 2),
    prim("&", do_and, 2),
    prim("|", do_or, 2),
    prim("!", do_not, 1),
    special("&&", do_and2, 2),
    special("||", do_or2, 2),
    prim("abs", do_abs, 1),
    prim("floor", do_floor, 1),
    prim("ceiling", do_ceiling, 1),
    prim("sqrt", do_sqrt, 1),
    prim("sign", do_sign, 1),
    prim("trunc", do_trunc, -1),
    prim("exp", do_exp, 1),
    prim("expm1", do_expm1, 1),
    prim("log1p", do_log1p, 1),
    prim("cos", do_cos, 1),
    prim("sin", do_sin, 1),
    prim("tan", do_tan, 1),
    prim("cospi", do_cospi, 1),
    prim("sinpi", do_sinpi, 1),
    prim("tanpi", do_tanpi, 1),
    prim("acos", do_acos, 1),
    prim("asin", do_asin, 1),
    prim("atan", do_atan, 1),
    prim("cosh", do_cosh, 1),
    prim("sinh", do_sinh, 1),
    prim("tanh", do_tanh, 1),
    prim("acosh", do_acosh, 1),
    prim("asinh", do_asinh, 1),
    prim("atanh", do_atanh, 1),
    prim("gamma", do_gamma, 1),
    prim("lgamma", do_lgamma, 1),
    prim("digamma", do_digamma, 1),
    prim("trigamma", do_trigamma, 1),
    special("log", do_log, -1),
    prim("log2", do_log2, 1),
    prim("log10", do_log10, 1),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArithOp {
    Plus,
    Minus,
    Times,
    Div,
    Pow,
    Mod,
    IDiv,
}

impl ArithOp {
    fn name(self) -> &'static str {
        match self {
            ArithOp::Plus => "+",
            ArithOp::Minus => "-",
            ArithOp::Times => "*",
            ArithOp::Div => "/",
            ArithOp::Pow => "^",
            ArithOp::Mod => "%%",
            ArithOp::IDiv => "%/%",
        }
    }
}

// ----------------------------------------------------------------------
// scalar kernels

pub use robigo_numerics::arith::{fmod_warns, myfloor, myfmod};

pub fn r_pow(x: f64, y: f64) -> f64 {
    if y == 2.0 {
        return x * x;
    }
    if x == 1.0 || y == 0.0 {
        return 1.0;
    }
    if x == 0.0 {
        if y > 0.0 {
            return 0.0;
        } else if y < 0.0 {
            return f64::INFINITY;
        } else {
            return y;
        }
    }
    if x.is_finite() && y.is_finite() {
        return x.powf(y);
    }
    if x.is_nan() || y.is_nan() {
        // R: arithmetic.c:R_pow's `x + y` as gcc compiles it: the addsd
        // destination holds y, so y's NaN payload wins (observable:
        // (1+NA)^(-(0/0)) is NaN, (-(0/0))^(1+NA) is NA).
        return super::complex::sse::add(y, x);
    }
    if !x.is_finite() {
        if x > 0.0 {
            return if y < 0.0 { 0.0 } else { f64::INFINITY };
        } else if y.is_finite() && y == y.floor() {
            return if y < 0.0 {
                0.0
            } else if myfmod(y, 2.0) != 0.0 {
                x
            } else {
                -x
            };
        }
    }
    if !y.is_finite() && x >= 0.0 {
        if y > 0.0 {
            return if x >= 1.0 { f64::INFINITY } else { 0.0 };
        } else {
            return if x < 1.0 { f64::INFINITY } else { 0.0 };
        }
    }
    f64::NAN
}

// Inline across codegen units: the VM's scalar fast path calls this for
// every double operation, and LLVM stopped inlining it there when this
// module grew.
#[inline]
pub fn real_op(op: ArithOp, a: f64, b: f64) -> f64 {
    match op {
        ArithOp::Plus => a + b,
        ArithOp::Minus => a - b,
        ArithOp::Times => a * b,
        ArithOp::Div => a / b,
        ArithOp::Pow => r_pow(a, b),
        ArithOp::Mod => myfmod(a, b),
        ArithOp::IDiv => myfloor(a, b),
    }
}

#[inline]
pub fn int_to_real(x: i32) -> f64 {
    if x == NA_INTEGER { NA_REAL } else { x as f64 }
}

// ----------------------------------------------------------------------
// R_binary

fn is_ts(v: &Value) -> bool {
    v.get_attr(s::TSP).is_some()
}

fn conformable(x: &Value, y: &Value) -> bool {
    match (x.get_attr(s::DIM), y.get_attr(s::DIM)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => a.data == b.data,
        _ => false,
    }
}

/// Prepare an operand: NULL becomes integer(0); non-numeric is an error.
fn fixup(v: Value, call: &Value) -> RResult<Value> {
    match v {
        Value::Null => Ok(Value::ints(vec![])),
        Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) => Ok(v),
        _ => Err(Flow::error_call(call.clone(), "non-numeric argument to binary operator")),
    }
}

/// Attribute handling shared by arithmetic, comparison and logic
/// (the tail of R_binary / relop / lbinary).
struct BinAttrs {
    dims: Option<Value>,
    xnames: Option<Value>,
    ynames: Option<Value>,
    any_attr: bool,
}

fn binary_attrs(x: &Value, y: &Value, call: &Value) -> RResult<BinAttrs> {
    let nx = x.len();
    let ny = y.len();
    let xattr = x.has_attrs();
    let yattr = y.has_attrs();
    let xarray = x.get_attr(s::DIM).is_some();
    let yarray = y.get_attr(s::DIM).is_some();
    let mut dims = None;
    let (xnames, ynames);
    if xarray || yarray {
        if xarray && yarray {
            if !conformable(x, y) {
                return Err(Flow::error_call(call.clone(), "non-conformable arrays"));
            }
            dims = x.get_attr(s::DIM);
        } else if xarray && (ny != 0 || nx == 0) {
            dims = x.get_attr(s::DIM);
        } else if yarray && (nx != 0 || ny == 0) {
            dims = y.get_attr(s::DIM);
        }
        xnames = if xattr { x.get_attr(s::DIMNAMES) } else { None };
        ynames = if yattr { y.get_attr(s::DIMNAMES) } else { None };
    } else {
        xnames = if xattr { Some(get_attrib(x, s::NAMES)).filter(|v| !v.is_null()) } else { None };
        ynames = if yattr { Some(get_attrib(y, s::NAMES)).filter(|v| !v.is_null()) } else { None };
    }
    Ok(BinAttrs { dims, xnames, ynames, any_attr: xattr || yattr })
}

fn apply_bin_attrs(val: &mut Value, a: BinAttrs) -> RResult<()> {
    if !a.any_attr {
        return Ok(());
    }
    let n = val.len();
    if let Some(d) = a.dims {
        // R: setAttrib(val, R_DimSymbol, dims) routes through dimgets,
        // whose product check fires when the recycled result is longer
        // than the array operand (e.g. `m + 1:16`): "dims [product %d]
        // do not match the length of object [%d]".
        *val = set_attrib(std::mem::replace(val, Value::Null), s::DIM, d)?;
        if let Some(dn) = a.xnames {
            val.set_attr_raw(s::DIMNAMES, dn);
        } else if let Some(dn) = a.ynames {
            val.set_attr_raw(s::DIMNAMES, dn);
        }
    } else if let Some(xn) = a.xnames.filter(|xn| xn.len() == n) {
        val.set_attr_raw(s::NAMES, xn);
    } else if let Some(yn) = a.ynames.filter(|yn| yn.len() == n) {
        val.set_attr_raw(s::NAMES, yn);
    }
    Ok(())
}

/// R: arithmetic.c:R_binary, relop.c:do_relop_dflt, logic.c:lbinary —
/// the time-series operand: an error when the only ts operand is the
/// shorter one (ERROR_TSVEC_MISMATCH), else the tsp and class to give
/// the result (x's when both are; Null when the operand lacks the
/// attribute, so it is removed from the result).
fn ts_operand(x: &Value, y: &Value, call: &Value) -> RResult<Option<(Value, Value)>> {
    let (xts, yts) = (is_ts(x), is_ts(y));
    if !xts && !yts {
        return Ok(None);
    }
    if xts != yts && (if xts { x.len() < y.len() } else { y.len() < x.len() }) {
        return Err(Flow::error_call(call.clone(), "time-series/vector length mismatch"));
    }
    let src = if xts { x } else { y };
    Ok(Some((src.get_attr(s::TSP).unwrap_or(Value::Null), src.get_attr(s::CLASS).unwrap_or(Value::Null))))
}

/// R: "must set *after* dims!" — through the validating setters (tspgets
/// checks the parameters and rejects zero-length results; classgets).
fn apply_ts_attrs(val: &mut Value, ts: Option<(Value, Value)>) -> RResult<()> {
    if let Some((tsp, klass)) = ts {
        if !tsp.is_null() {
            *val = set_attrib(std::mem::replace(val, Value::Null), s::TSP, tsp)?;
        }
        *val = set_attrib(std::mem::replace(val, Value::Null), s::CLASS, klass)?;
    }
    Ok(())
}

/// Copy attributes from the operands (as in real_binary: the longer
/// operand's attributes, x's taking precedence).
fn copy_operand_attrs(x: &Value, y: &Value, val: &mut Value) {
    let n = val.len();
    if y.has_attrs() && y.len() == n {
        copy_most_attrib(y, val);
    }
    if x.has_attrs() && x.len() == n {
        copy_most_attrib(x, val);
    }
}

fn check_recycling(interp: &mut Interp, call: &Value, nx: usize, ny: usize) -> RResult<()> {
    if nx > 0 && ny > 0 && (if nx > ny { nx % ny } else { ny % nx }) != 0 {
        interp.warningcall(call, "longer object length is not a multiple of shorter object length")?;
    }
    Ok(())
}

/// Fast path: attribute-free scalars of type double (or int with double).
#[inline]
fn scalar_arith(op: ArithOp, x: &Value, y: &Value) -> Option<Value> {
    let a = match x {
        Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => v.data[0],
        Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() && matches!(y, Value::Real(_)) => {
            int_to_real(v.data[0])
        }
        _ => return None,
    };
    let b = match y {
        Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => v.data[0],
        Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() => int_to_real(v.data[0]),
        _ => return None,
    };
    if op == ArithOp::Mod && fmod_warns(a, b) {
        return None;
    }
    // R: arithmetic.c:do_arith's scalar fast path: with an integer operand
    // the compiled + and * keep y's NaN payload, with two doubles x's.
    let r = match op {
        ArithOp::Plus | ArithOp::Times => {
            let mul = op == ArithOp::Times;
            if matches!(x, Value::Int(_)) || matches!(y, Value::Int(_)) {
                exact::one(mul, b, a)
            } else {
                exact::one(mul, a, b)
            }
        }
        _ => real_op(op, a, b),
    };
    Some(Value::real(r))
}

pub fn arith_binary(interp: &mut Interp, call: &Value, op: ArithOp, x: Value, y: Value) -> RResult<Value> {
    if let Some(v) = scalar_arith(op, &x, &y) {
        return Ok(v);
    }
    let mut x = fixup(x, call)?;
    let mut y = fixup(y, call)?;
    let (nx, ny) = (x.len(), y.len());
    let xarray = x.get_attr(s::DIM).is_some();
    let yarray = y.get_attr(s::DIM).is_some();
    if xarray != yarray {
        if xarray && nx == 1 && ny != 1 {
            if ny != 0 {
                interp.warningcall(
                    call,
                    "Recycling array of length 1 in array-vector arithmetic is deprecated.\n  Use c() or as.vector() instead.",
                )?;
            }
            x.set_attr_raw(s::DIM, Value::Null);
        }
        if yarray && ny == 1 && nx != 1 {
            if nx != 0 {
                interp.warningcall(
                    call,
                    "Recycling array of length 1 in vector-array arithmetic is deprecated.\n  Use c() or as.vector() instead.",
                )?;
            }
            y.set_attr_raw(s::DIM, Value::Null);
        }
    }
    let attrs = binary_attrs(&x, &y, call)?;
    let ts = ts_operand(&x, &y, call)?;
    check_recycling(interp, call, nx, ny)?;
    let n = if nx == 0 || ny == 0 { 0 } else { nx.max(ny) };
    let mut val = match (&x, &y) {
        (Value::Cplx(_), _) | (_, Value::Cplx(_)) => {
            let mut w = CoerceWarnings::default();
            let a = cplx_view(&x, &mut w);
            let b = cplx_view(&y, &mut w);
            Value::cplxs(complex_binary(op, &a, &b, n)?)
        }
        (Value::Real(_), _) | (_, Value::Real(_)) => {
            if op == ArithOp::Mod {
                let (a, b) = (real_view(&x), real_view(&y));
                let k = (0..n).filter(|&i| fmod_warns(a[i % a.len()], b[i % b.len()])).count();
                for _ in 0..k {
                    interp.warning("probable complete loss of accuracy in modulus")?;
                }
            }
            if !attrs.any_attr {
                // Plain vectors: compute into an operand nobody else holds.
                if let Some(out) = real_binary_inplace(op, &mut x, &mut y, n) {
                    return Ok(Value::reals(out));
                }
            }
            let int_operand = matches!(x, Value::Int(_)) || matches!(y, Value::Int(_));
            let a = real_view(&x);
            let b = real_view(&y);
            Value::reals(real_binary(op, &a, &b, n, int_operand))
        }
        _ => integer_binary(interp, call, op, &x, &y, n)?,
    };
    if !attrs.any_attr {
        return Ok(val);
    }
    copy_operand_attrs(&x, &y, &mut val);
    apply_bin_attrs(&mut val, attrs)?;
    apply_ts_attrs(&mut val, ts)?;
    Ok(val)
}

/// Apply `f` elementwise with R's recycling: one operand has length `n`
/// or both are recycled. Written as plain iterator maps per operator so
/// the loops vectorize; results are the same IEEE operations as R's.
macro_rules! real_kernel {
    ($a:expr, $b:expr, $n:expr, $f:expr) => {{
        let (a, b, n) = ($a, $b, $n);
        #[allow(unused_mut)]
        let mut f = $f;
        if n == 0 {
            Vec::new()
        } else if a.len() == n && b.len() == n {
            a.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)).collect::<Vec<_>>()
        } else if b.len() == 1 && a.len() == n {
            let y = b[0];
            a.iter().map(|&x| f(x, y)).collect()
        } else if a.len() == 1 && b.len() == n {
            let x = a[0];
            b.iter().map(|&y| f(x, y)).collect()
        } else if a.len() == n && n % b.len() == 0 {
            // `b` recycled a whole number of times (a matrix times a column).
            let mut out = Vec::with_capacity(n);
            for chunk in a.chunks(b.len()) {
                out.extend(chunk.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)));
            }
            out
        } else if b.len() == n && n % a.len() == 0 {
            let mut out = Vec::with_capacity(n);
            for chunk in b.chunks(a.len()) {
                out.extend(a.iter().zip(chunk.iter()).map(|(&x, &y)| f(x, y)));
            }
            out
        } else {
            let (na, nb) = (a.len(), b.len());
            let (mut i, mut j) = (0, 0);
            (0..n)
                .map(|_| {
                    let r = f(a[i], b[j]);
                    i = if i + 1 == na { 0 } else { i + 1 };
                    j = if j + 1 == nb { 0 } else { j + 1 };
                    r
                })
                .collect()
        }
    }};
}

/// `a op b` on doubles (R: arithmetic.c:real_binary).
fn real_binary(op: ArithOp, a: &[f64], b: &[f64], n: usize, int_operand: bool) -> Vec<f64> {
    match op {
        ArithOp::Plus => add_mul(false, a, b, n, int_operand),
        ArithOp::Minus => real_kernel!(a, b, n, |x: f64, y: f64| x - y),
        ArithOp::Times => add_mul(true, a, b, n, int_operand),
        ArithOp::Div => real_kernel!(a, b, n, |x: f64, y: f64| x / y),
        ArithOp::Pow => real_kernel!(a, b, n, r_pow),
        ArithOp::Mod => real_kernel!(a, b, n, myfmod),
        ArithOp::IDiv => real_kernel!(a, b, n, myfloor),
    }
}

/// [`real_binary`] writing into the buffer of an operand of length `n`
/// that is a double vector held only here (a temporary), as R reuses
/// unreferenced arguments. `None` if neither operand qualifies.
fn real_binary_inplace(op: ArithOp, x: &mut Value, y: &mut Value, n: usize) -> Option<Vec<f64>> {
    fn take_unique(v: &mut Value, n: usize) -> Option<Vec<f64>> {
        match v {
            Value::Real(rc) if rc.data.len() == n => Rc::get_mut(rc).map(|v| std::mem::take(&mut v.data)),
            _ => None,
        }
    }
    macro_rules! apply {
        ($out:expr, $other:expr, $left:expr) => {{
            let mut out: Vec<f64> = $out;
            let other = real_view($other);
            let m = other.len();
            macro_rules! go {
                ($f:expr) => {{
                    let f = $f;
                    if m == n {
                        for (o, &w) in out.iter_mut().zip(other.iter()) {
                            *o = if $left { f(*o, w) } else { f(w, *o) };
                        }
                    } else if m == 1 {
                        let w = other[0];
                        for o in out.iter_mut() {
                            *o = if $left { f(*o, w) } else { f(w, *o) };
                        }
                    } else {
                        for (i, o) in out.iter_mut().enumerate() {
                            let w = other[i % m];
                            *o = if $left { f(*o, w) } else { f(w, *o) };
                        }
                    }
                }};
            }
            match op {
                ArithOp::Plus | ArithOp::Times => {
                    use exact::{in_place_k, in_place_v};
                    let mul = op == ArithOp::Times;
                    // `out` (a double) holds x if `$left`; the other operand
                    // may be an integer.
                    let int_operand = matches!($other, Value::Int(_));
                    let (nx, ny) = if $left { (n, m) } else { (m, n) };
                    let x_dest = x_is_dest(int_operand, nx, ny);
                    let own_dest = if $left { x_dest } else { !x_dest };
                    if m == n {
                        exact::dispatch!(in_place_v, mul, own_dest, &mut out, &other[..]);
                    } else if m == 1 {
                        exact::dispatch!(in_place_k, mul, own_dest, &mut out, other[0]);
                    } else {
                        for (i, o) in out.iter_mut().enumerate() {
                            let w = other[i % m];
                            *o = if own_dest { exact::one(mul, *o, w) } else { exact::one(mul, w, *o) };
                        }
                    }
                }
                ArithOp::Minus => go!(|p: f64, q: f64| p - q),
                ArithOp::Div => go!(|p: f64, q: f64| p / q),
                ArithOp::Pow => go!(r_pow),
                ArithOp::Mod => go!(myfmod),
                ArithOp::IDiv => go!(myfloor),
            }
            Some(out)
        }};
    }
    if n == 0 {
        return None;
    }
    if let Some(out) = take_unique(x, n) {
        return apply!(out, &*y, true);
    }
    if let Some(out) = take_unique(y, n) {
        return apply!(out, &*x, false);
    }
    None
}

fn real_view(v: &Value) -> std::borrow::Cow<'_, [f64]> {
    match v {
        Value::Real(r) => std::borrow::Cow::Borrowed(r.data.as_slice()),
        Value::Int(iv) | Value::Lgl(iv) => std::borrow::Cow::Owned(iv.data.iter().map(|&i| int_to_real(i)).collect()),
        _ => std::borrow::Cow::Owned(Vec::new()),
    }
}

fn int_slice(v: &Value) -> &[i32] {
    match v {
        Value::Int(x) | Value::Lgl(x) => &x.data,
        _ => &[],
    }
}

fn integer_binary(interp: &mut Interp, call: &Value, op: ArithOp, x: &Value, y: &Value, n: usize) -> RResult<Value> {
    let a = int_slice(x);
    let b = int_slice(y);
    let mut naflag = false;
    // R_integer_plus/minus/times: NA in, NA out; results outside
    // [-INT_MAX, INT_MAX] are NA with a warning.
    macro_rules! checked {
        ($f:expr) => {{
            let f = $f;
            let mut ovf = false;
            let out: Vec<i32> = real_kernel!(a, b, n, |x: i32, y: i32| {
                let r: i64 = f(x as i64, y as i64);
                let na = x == NA_INTEGER || y == NA_INTEGER;
                let bad = r > i32::MAX as i64 || r < -(i32::MAX as i64);
                ovf |= bad & !na;
                if na | bad { NA_INTEGER } else { r as i32 }
            });
            naflag = ovf;
            Value::ints(out)
        }};
    }
    let val = match op {
        ArithOp::Plus => checked!(|x: i64, y: i64| x + y),
        ArithOp::Minus => checked!(|x: i64, y: i64| x - y),
        ArithOp::Times => checked!(|x: i64, y: i64| x * y),
        ArithOp::Div => Value::reals(real_kernel!(a, b, n, |x1: i32, x2: i32| {
            if x1 == NA_INTEGER || x2 == NA_INTEGER { NA_REAL } else { x1 as f64 / x2 as f64 }
        })),
        ArithOp::Pow => Value::reals(real_kernel!(a, b, n, |x1: i32, x2: i32| {
            if x1 == 1 || x2 == 0 {
                1.0
            } else if x1 == NA_INTEGER || x2 == NA_INTEGER {
                NA_REAL
            } else {
                r_pow(x1 as f64, x2 as f64)
            }
        })),
        ArithOp::Mod => Value::ints(real_kernel!(a, b, n, |x1: i32, x2: i32| {
            if x1 == NA_INTEGER || x2 == NA_INTEGER || x2 == 0 {
                NA_INTEGER
            } else if x1 >= 0 && x2 > 0 {
                x1 % x2
            } else {
                myfmod(x1 as f64, x2 as f64) as i32
            }
        })),
        ArithOp::IDiv => Value::ints(real_kernel!(a, b, n, |x1: i32, x2: i32| {
            if x1 == NA_INTEGER || x2 == NA_INTEGER || x2 == 0 {
                NA_INTEGER
            } else {
                (x1 as f64 / x2 as f64).floor() as i32
            }
        })),
    };
    if naflag {
        interp.warningcall(call, "NAs produced by integer overflow")?;
    }
    Ok(val)
}

/// `a op b` on complex vectors with R's recycling (R:
/// complex.c:complex_binary).
pub(crate) fn complex_binary(op: ArithOp, a: &[Complex], b: &[Complex], n: usize) -> RResult<Vec<Complex>> {
    use super::complex::{cadd, cdiv, cmul, cpow, csub, zip_unrolled};
    Ok(match op {
        // GCC commuted the compiled adds (the second operand's NaN payload
        // wins) and calls __muldc3 with (y, x); see complex.rs.
        ArithOp::Plus if a.len() == n && b.len() == n => zip_unrolled(a, b, cadd),
        ArithOp::Plus => real_kernel!(a, b, n, cadd),
        ArithOp::Minus if a.len() == n && b.len() == n => zip_unrolled(a, b, csub),
        ArithOp::Minus => real_kernel!(a, b, n, csub),
        // C99 `*`/`/` as compiled to libgcc's __muldc3/__divdc3.
        ArithOp::Times => real_kernel!(a, b, n, |x: Complex, y: Complex| cmul(y, x)),
        ArithOp::Div => real_kernel!(a, b, n, cdiv),
        ArithOp::Pow => real_kernel!(a, b, n, cpow),
        ArithOp::Mod | ArithOp::IDiv if n == 0 => Vec::new(),
        ArithOp::Mod | ArithOp::IDiv => return Err(Flow::error("unimplemented complex operation")),
    })
}

/// The complex values of an arithmetic operand: borrowed when it is
/// complex already, coerced otherwise.
fn cplx_view<'a>(v: &'a Value, w: &mut CoerceWarnings) -> std::borrow::Cow<'a, [Complex]> {
    match v {
        Value::Cplx(c) => std::borrow::Cow::Borrowed(c.data.as_slice()),
        _ => std::borrow::Cow::Owned(crate::coerce::to_cplx(v, w)),
    }
}

pub fn arith_unary(call: &Value, op: ArithOp, x: Value) -> RResult<Value> {
    match x {
        Value::Lgl(v) => {
            let data: Vec<i32> =
                v.data.iter().map(|&e| if e == NA_INTEGER || op == ArithOp::Plus { e } else { -e }).collect();
            let mut out = Value::ints(data);
            if let Some(a) = &v.attrs {
                out.set_attrs(Some(a.clone()));
            }
            Ok(out)
        }
        Value::Int(mut v) => {
            if op == ArithOp::Minus {
                for e in Rc::make_mut(&mut v).data.iter_mut() {
                    if *e != NA_INTEGER {
                        *e = -*e;
                    }
                }
            }
            Ok(Value::Int(v))
        }
        Value::Real(mut v) => {
            if op == ArithOp::Minus {
                for e in Rc::make_mut(&mut v).data.iter_mut() {
                    *e = -*e;
                }
            }
            Ok(Value::Real(v))
        }
        Value::Cplx(mut v) => {
            if op == ArithOp::Minus {
                for e in Rc::make_mut(&mut v).data.iter_mut() {
                    *e = Complex::new(-e.re, -e.im);
                }
            }
            Ok(Value::Cplx(v))
        }
        _ => Err(Flow::error_call(call.clone(), "invalid argument to unary operator")),
    }
}

fn do_arith(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env, op: ArithOp) -> RResult<Value> {
    if args.iter().take(2).any(|a| a.value.is_object()) {
        if let Some(r) = i.try_dispatch_group("Ops", op.name(), call, &args, env)? {
            return Ok(r);
        }
    }
    match args.len() {
        2 => {
            let y = args.pop().unwrap().value;
            let x = args.pop().unwrap().value;
            arith_binary(i, call, op, x, y)
        }
        1 if matches!(op, ArithOp::Plus | ArithOp::Minus) => arith_unary(call, op, args.pop().unwrap().value),
        n => Err(Flow::error_call(
            call.clone(),
            format!("operator needs one or two arguments{}", if n == 0 { "" } else { "" }),
        )),
    }
}

fn do_plus(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Plus)
}
fn do_minus(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Minus)
}
fn do_times(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Times)
}
fn do_div(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Div)
}
fn do_pow(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Pow)
}
fn do_mod(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::Mod)
}
fn do_idiv(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_arith(i, c, a, e, ArithOp::IDiv)
}

// ----------------------------------------------------------------------
// comparison (relop.c)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl RelOp {
    fn name(self) -> &'static str {
        match self {
            RelOp::Eq => "==",
            RelOp::Ne => "!=",
            RelOp::Lt => "<",
            RelOp::Le => "<=",
            RelOp::Gt => ">",
            RelOp::Ge => ">=",
        }
    }
    pub fn test<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            RelOp::Eq => a == b,
            RelOp::Ne => a != b,
            RelOp::Lt => a < b,
            RelOp::Le => a <= b,
            RelOp::Gt => a > b,
            RelOp::Ge => a >= b,
        }
    }
}

fn lang_to_str(v: &Value) -> Value {
    match v {
        Value::Sym(sy) => Value::strs(vec![Some(sy.name())]),
        Value::Lang(_) => Value::str(&crate::deparse::deparse_one_line(v)),
        _ => v.clone(),
    }
}

pub fn relop(interp: &mut Interp, call: &Value, op: RelOp, x: Value, y: Value) -> RResult<Value> {
    let mut x = lang_to_str(&x);
    let mut y = lang_to_str(&y);
    if x.is_null() {
        x = Value::ints(vec![]);
    }
    if y.is_null() {
        y = Value::ints(vec![]);
    }
    let ok = |v: &Value| v.is_atomic() || matches!(v, Value::List(_) | Value::Expr(_));
    if !ok(&x) || !ok(&y) {
        return Err(Flow::error_call(
            call.clone(),
            format!("comparison ({}) is possible only for atomic and list types", op.name()),
        ));
    }
    // R: relop.c:do_relop_dflt's coercion chain — a list or expression
    // operand coerces to the other operand's type; the gate is
    // isVectorizable (a plain error; an expression never qualifies, and
    // character is exempt through the deparse path). Two list operands
    // match no branch of the chain.
    let is_list = |v: &Value| matches!(v, Value::List(_) | Value::Expr(_));
    if is_list(&x) && is_list(&y) {
        return Err(Flow::error_call(call.clone(), "comparison of these types is not implemented"));
    }
    let other_type = |v: &Value| match v {
        Value::Str(_) => VType::Str,
        Value::Cplx(_) => VType::Cplx,
        Value::Real(_) => VType::Real,
        Value::Int(_) => VType::Int,
        Value::Lgl(_) => VType::Lgl,
        Value::Raw(_) => VType::Raw,
        _ => unreachable!("one operand is atomic here"),
    };
    if is_list(&x) && !matches!(y, Value::Str(_)) {
        let t = other_type(&y);
        let mut w = CoerceWarnings::default();
        x = super::coerce::relop_list_coerce(&x, t, &mut w)?;
        w.emit(interp)?;
    }
    if is_list(&y) && !matches!(x, Value::Str(_)) {
        let t = other_type(&x);
        let mut w = CoerceWarnings::default();
        y = super::coerce::relop_list_coerce(&y, t, &mut w)?;
        w.emit(interp)?;
    }
    let (nx, ny) = (x.len(), y.len());
    let attrs = binary_attrs(&x, &y, call)?;
    let ts = ts_operand(&x, &y, call)?;
    check_recycling(interp, call, nx, ny)?;
    let n = if nx == 0 || ny == 0 { 0 } else { nx.max(ny) };
    // R: relop.c — a raw operand compared with a real, integer or logical
    // one is coerced to that type (real first, then integer, then logical)
    if matches!(x, Value::Raw(_)) != matches!(y, Value::Raw(_)) {
        let other = if matches!(x, Value::Raw(_)) { &y } else { &x };
        let t = match other {
            Value::Real(_) => Some(VType::Real),
            Value::Int(_) => Some(VType::Int),
            Value::Lgl(_) => Some(VType::Lgl),
            _ => None,
        };
        if let Some(t) = t {
            if matches!(x, Value::Raw(_)) {
                x = crate::coerce::coerce_quiet(&x, t);
            } else {
                y = crate::coerce::coerce_quiet(&y, t);
            }
        }
    }
    let out: Vec<Lgl> = if matches!(x, Value::Str(_)) || matches!(y, Value::Str(_)) {
        let a = crate::coerce::to_str(&x);
        let b = crate::coerce::to_str(&y);
        (0..n)
            .map(|i| match (&a[i % nx], &b[i % ny]) {
                (Some(p), Some(q)) => op.test(scollate(p, q), std::cmp::Ordering::Equal) as Lgl,
                _ => NA_LOGICAL,
            })
            .collect()
    } else if matches!(x, Value::Cplx(_)) || matches!(y, Value::Cplx(_)) {
        if !matches!(op, RelOp::Eq | RelOp::Ne) {
            return Err(Flow::error_call(call.clone(), "invalid comparison with complex values"));
        }
        let mut w = CoerceWarnings::default();
        let a = crate::coerce::to_cplx(&x, &mut w);
        let b = crate::coerce::to_cplx(&y, &mut w);
        (0..n)
            .map(|i| {
                let (p, q) = (a[i % nx], b[i % ny]);
                if p.is_na() || q.is_na() {
                    NA_LOGICAL
                } else {
                    let eq = p.re == q.re && p.im == q.im;
                    (if op == RelOp::Eq { eq } else { !eq }) as Lgl
                }
            })
            .collect()
    } else if matches!(x, Value::Real(_)) || matches!(y, Value::Real(_)) {
        let a = real_view(&x);
        let b = real_view(&y);
        macro_rules! cmp {
            ($t:tt) => {
                real_kernel!(&*a, &*b, n, |p: f64, q: f64| {
                    let na = p.is_nan() | q.is_nan();
                    let r = (p $t q) as Lgl;
                    if na { NA_LOGICAL } else { r }
                })
            };
        }
        match op {
            RelOp::Eq => cmp!(==),
            RelOp::Ne => cmp!(!=),
            RelOp::Lt => cmp!(<),
            RelOp::Le => cmp!(<=),
            RelOp::Gt => cmp!(>),
            RelOp::Ge => cmp!(>=),
        }
    } else if matches!(x, Value::Raw(_)) && matches!(y, Value::Raw(_)) {
        let (Value::Raw(a), Value::Raw(b)) = (&x, &y) else { unreachable!() };
        (0..n).map(|i| op.test(a.data[i % nx], b.data[i % ny]) as Lgl).collect()
    } else {
        let mut w = CoerceWarnings::default();
        let (ca, cb);
        let a: &[i32] = match &x {
            Value::Int(v) | Value::Lgl(v) => &v.data,
            _ => {
                ca = crate::coerce::to_int(&x, &mut w);
                &ca
            }
        };
        let b: &[i32] = match &y {
            Value::Int(v) | Value::Lgl(v) => &v.data,
            _ => {
                cb = crate::coerce::to_int(&y, &mut w);
                &cb
            }
        };
        macro_rules! cmp {
            ($t:tt) => {
                real_kernel!(a, b, n, |p: i32, q: i32| {
                    let na = (p == NA_INTEGER) | (q == NA_INTEGER);
                    let r = (p $t q) as Lgl;
                    if na { NA_LOGICAL } else { r }
                })
            };
        }
        match op {
            RelOp::Eq => cmp!(==),
            RelOp::Ne => cmp!(!=),
            RelOp::Lt => cmp!(<),
            RelOp::Le => cmp!(<=),
            RelOp::Gt => cmp!(>),
            RelOp::Ge => cmp!(>=),
        }
    };
    let mut val = Value::lgls(out);
    apply_bin_attrs(&mut val, attrs)?;
    apply_ts_attrs(&mut val, ts)?;
    Ok(val)
}

/// String collation. robigo currently implements the C locale (byte order).
pub fn scollate(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

fn do_relop(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env, op: RelOp) -> RResult<Value> {
    if args.iter().take(2).any(|a| a.value.is_object()) {
        if let Some(r) = i.try_dispatch_group("Ops", op.name(), call, &args, env)? {
            return Ok(r);
        }
    }
    if args.len() != 2 {
        return Err(Flow::error("operator needs two arguments"));
    }
    let y = args.pop().unwrap().value;
    let x = args.pop().unwrap().value;
    // Factors compare via Ops.factor; reaching here with one means no method.
    relop(i, call, op, x, y)
}

fn do_eq(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Eq)
}
fn do_ne(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Ne)
}
fn do_lt(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Lt)
}
fn do_le(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Le)
}
fn do_gt(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Gt)
}
fn do_ge(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_relop(i, c, a, e, RelOp::Ge)
}

// ----------------------------------------------------------------------
// logic (logic.c)

/// `v` as logicals: borrowed when it is a logical vector.
fn lgl_view(v: &Value) -> std::borrow::Cow<'_, [Lgl]> {
    match v {
        Value::Lgl(x) => std::borrow::Cow::Borrowed(&x.data),
        _ => {
            let mut w = CoerceWarnings::default();
            std::borrow::Cow::Owned(crate::coerce::to_lgl(v, &mut w))
        }
    }
}

fn do_logic(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env, and: bool) -> RResult<Value> {
    let name = if and { "&" } else { "|" };
    if args.iter().take(2).any(|a| a.value.is_object()) {
        if let Some(r) = i.try_dispatch_group("Ops", name, call, &args, env)? {
            return Ok(r);
        }
    }
    if args.len() != 2 {
        // R's checkArity (logic.c:do_logic): the singular form has no
        // 's' on 'argument'.
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires 2",
                args.len(),
                if args.len() == 1 { "" } else { "s" },
                name
            ),
        ));
    }
    let mut y = args.pop().unwrap().value;
    let mut x = args.pop().unwrap().value;
    if x.is_null() {
        x = Value::lgls(vec![]);
    }
    if y.is_null() {
        y = Value::lgls(vec![]);
    }
    let okt = |v: &Value| matches!(v, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Raw(_));
    if !okt(&x) || !okt(&y) {
        return Err(Flow::error_call(
            call.clone(),
            format!("operations are possible only for numeric, logical or complex types"),
        ));
    }
    let (nx, ny) = (x.len(), y.len());
    let attrs = binary_attrs(&x, &y, call)?;
    let ts = ts_operand(&x, &y, call)?;
    check_recycling(i, call, nx, ny)?;
    let n = if nx == 0 || ny == 0 { 0 } else { nx.max(ny) };
    if let (Value::Raw(a), Value::Raw(b)) = (&x, &y) {
        let out: Vec<u8> = (0..n)
            .map(|k| if and { a.data[k % nx] & b.data[k % ny] } else { a.data[k % nx] | b.data[k % ny] })
            .collect();
        let mut val = Value::raws(out);
        apply_bin_attrs(&mut val, attrs)?;
        apply_ts_attrs(&mut val, ts)?;
        return Ok(val);
    }
    if matches!(x, Value::Raw(_)) || matches!(y, Value::Raw(_)) {
        return Err(Flow::error_call(
            call.clone(),
            "operations are possible only for numeric, logical or complex types",
        ));
    }
    let a = lgl_view(&x);
    let b = lgl_view(&y);
    let (a, b): (&[Lgl], &[Lgl]) = (&a, &b);
    let out: Vec<Lgl> = if and {
        real_kernel!(a, b, n, |p: Lgl, q: Lgl| if p == FALSE || q == FALSE {
            FALSE
        } else if p == NA_LOGICAL || q == NA_LOGICAL {
            NA_LOGICAL
        } else {
            TRUE
        })
    } else {
        real_kernel!(a, b, n, |p: Lgl, q: Lgl| if p == TRUE || q == TRUE {
            TRUE
        } else if p == NA_LOGICAL || q == NA_LOGICAL {
            NA_LOGICAL
        } else {
            FALSE
        })
    };
    let mut val = Value::lgls(out);
    apply_bin_attrs(&mut val, attrs)?;
    apply_ts_attrs(&mut val, ts)?;
    Ok(val)
}

fn do_and(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_logic(i, c, a, e, true)
}
fn do_or(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_logic(i, c, a, e, false)
}

fn do_not(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.first().is_some_and(|a| a.value.is_object()) {
        if let Some(r) = i.try_dispatch_group("Ops", "!", call, &args, env)? {
            return Ok(r);
        }
    }
    let x = args.pop().map(|a| a.value).unwrap_or(Value::Null);
    // R: logic.c:lunary — logical and raw arguments keep all attributes
    // (shallow_duplicate); numbers keep names, dim and dimnames.
    let keep_all_attrs = matches!(x, Value::Lgl(_) | Value::Raw(_));
    let names = get_attrib(&x, s::NAMES);
    let dim = x.get_attr(s::DIM);
    let dimnames = x.get_attr(s::DIMNAMES);
    let mut val = match &x {
        Value::Null => Value::lgls(vec![]),
        Value::Lgl(v) => {
            Value::lgls(v.data.iter().map(|&e| if e == NA_LOGICAL { e } else { (e == 0) as Lgl }).collect())
        }
        Value::Int(v) => {
            Value::lgls(v.data.iter().map(|&e| if e == NA_INTEGER { NA_LOGICAL } else { (e == 0) as Lgl }).collect())
        }
        Value::Real(v) => {
            Value::lgls(v.data.iter().map(|&e| if e.is_nan() { NA_LOGICAL } else { (e == 0.0) as Lgl }).collect())
        }
        Value::Cplx(v) => Value::lgls(
            v.data.iter().map(|e| if e.is_na() { NA_LOGICAL } else { (e.re == 0.0 && e.im == 0.0) as Lgl }).collect(),
        ),
        Value::Raw(v) => Value::raws(v.data.iter().map(|&b| !b).collect()),
        // R: logic.c:lunary — "for back-compatibility" a zero-length
        // argument of any other type gives logical(0), without attributes
        other if other.len() == 0 => return Ok(Value::lgls(vec![])),
        _ => return Err(Flow::error_call(call.clone(), "invalid argument type")),
    };
    if keep_all_attrs {
        if let Some(a) = x.attrs() {
            for (k, v) in &a.0 {
                val.set_attr_raw(*k, v.clone());
            }
        }
    } else {
        if !names.is_null() {
            val.set_attr_raw(s::NAMES, names);
        }
        if let Some(d) = dim {
            val.set_attr_raw(s::DIM, d);
        }
        if let Some(d) = dimnames {
            val.set_attr_raw(s::DIMNAMES, d);
        }
    }
    Ok(val)
}

/// `&&` / `||` operand: R's isNumber (logical, integer, double or
/// complex — not raw, not NULL, not string) and length <= 1
/// (R: logic.c:do_logic2, asLogical2 with checking).
pub fn scalar_logical(i: &mut Interp, call: &Value, v: &Value, which: &str, op: &str) -> RResult<Lgl> {
    let _ = i;
    if !matches!(v, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Cplx(_)) {
        return Err(Flow::error_call(call.clone(), format!("invalid '{which}' type in 'x {op} y'")));
    }
    if v.len() != 1 {
        if v.len() > 1 {
            return Err(Flow::error_call(call.clone(), format!("'length = {}' in coercion to 'logical(1)'", v.len())));
        }
        return Ok(NA_LOGICAL);
    }
    Ok(as_logical(v))
}

fn do_and2(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // R: logic.c:do_logic2 — plain error, the operator's name only.
    if args.len() != 2 {
        return Err(Flow::error("'&&' operator requires 2 arguments"));
    }
    let x = i.eval(&args[0].value, env)?;
    let a = scalar_logical(i, call, &x, "x", "&&")?;
    if a == FALSE {
        return Ok(Value::lgl(false));
    }
    let y = i.eval(&args[1].value, env)?;
    let b = scalar_logical(i, call, &y, "y", "&&")?;
    Ok(Value::lgl_raw(if a == TRUE && b == TRUE {
        TRUE
    } else if b == FALSE {
        FALSE
    } else {
        NA_LOGICAL
    }))
}

fn do_or2(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error("'||' operator requires 2 arguments"));
    }
    let x = i.eval(&args[0].value, env)?;
    let a = scalar_logical(i, call, &x, "x", "||")?;
    if a == TRUE {
        return Ok(Value::lgl(true));
    }
    let y = i.eval(&args[1].value, env)?;
    let b = scalar_logical(i, call, &y, "y", "||")?;
    Ok(Value::lgl_raw(if b == TRUE {
        TRUE
    } else if a == FALSE && b == FALSE {
        FALSE
    } else {
        NA_LOGICAL
    }))
}

// ----------------------------------------------------------------------
// Math group (do_math1)

fn math1<F: Fn(f64) -> f64>(
    i: &mut Interp,
    call: &Value,
    mut args: Vec<Arg>,
    env: &Env,
    name: &str,
    f: F,
) -> RResult<Value> {
    if args.len() != 1 {
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires 1",
                args.len(),
                if args.len() == 1 { "" } else { "s" },
                name
            ),
        ));
    }
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch_group("Math", name, call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    if matches!(x, Value::Cplx(_)) {
        return complex_math1(i, call, x, name);
    }
    if !r_is_numeric(x) {
        return Err(Flow::error_call(call.clone(), "non-numeric argument to mathematical function"));
    }
    // NaN in gives the same NaN out (keeping NA's payload); a new NaN
    // makes R warn.
    let g = |v: f64| if v.is_nan() { v } else { f(v) };
    let mut naflag = false;
    let unique = matches!(&args[0].value, Value::Real(rc) if Rc::strong_count(rc) == 1 && Rc::weak_count(rc) == 0);
    let val = match &mut args[0].value {
        // A double vector held only here: compute in place.
        Value::Real(rc) if unique => {
            let vecv = Rc::get_mut(rc).unwrap();
            for v in vecv.data.iter_mut() {
                let r = g(*v);
                naflag |= r.is_nan() && !v.is_nan();
                *v = r;
            }
            std::mem::replace(&mut args[0].value, Value::Null)
        }
        x => {
            let a = real_view(x);
            let out: Vec<f64> = a
                .iter()
                .map(|&v| {
                    let r = g(v);
                    naflag |= r.is_nan() && !v.is_nan();
                    r
                })
                .collect();
            let mut val = Value::reals(out);
            if let Some(at) = x.attrs() {
                val.set_attrs(Some(Box::new(at.clone())));
            }
            val
        }
    };
    if naflag {
        i.warningcall(call, "NaNs produced")?;
    }
    Ok(val)
}

macro_rules! math_fn {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
            math1(i, c, a, e, $rname, $f)
        }
    };
}

/// The complex kernels R's `complex_math1` implements (complex.c):
/// glibc's functions where R calls the C library, R's `z_*` wrappers
/// where it does not. `None` for the functions that error there
/// ("unimplemented complex function": floor, sign, gamma, cospi, ...).
fn math1_c_fn(name: &str) -> Option<fn(Complex) -> Complex> {
    match name {
        "log" => Some(super::complex::clog),
        "sqrt" => Some(super::complex::csqrt),
        "exp" => Some(super::complex::cexp),
        "cos" => Some(super::complex::ccos),
        "sin" => Some(super::complex::csin),
        "tan" => Some(super::complex::z_tan),
        "acos" => Some(super::complex::z_acos),
        "asin" => Some(super::complex::z_asin),
        "atan" => Some(super::complex::z_atan),
        "cosh" => Some(super::complex::ccosh),
        "sinh" => Some(super::complex::csinh),
        "tanh" => Some(super::complex::ctanh),
        "acosh" => Some(super::complex::z_acosh),
        "asinh" => Some(super::complex::z_asinh),
        "atanh" => Some(super::complex::z_atanh),
        _ => None,
    }
}

/// `cmath1`/`complex_math1` (R: complex.c): NA in gives complex NA; a
/// NaN produced warns against the call.
fn complex_math1(i: &mut Interp, call: &Value, x: &Value, name: &str) -> RResult<Value> {
    let f = math1_c_fn(name).ok_or_else(|| Flow::error_call(call.clone(), "unimplemented complex function"))?;
    let Value::Cplx(v) = x else { unreachable!() };
    let mut naflag = false;
    let out: Vec<Complex> = v
        .data
        .iter()
        .map(|&z| {
            if is_na_real(z.re) || is_na_real(z.im) {
                Complex::NA
            } else {
                let r = f(z);
                if (r.re.is_nan() || r.im.is_nan()) && !(z.re.is_nan() || z.im.is_nan()) {
                    naflag = true;
                }
                r
            }
        })
        .collect();
    let mut val = Value::cplxs(out);
    if let Some(a) = &v.attrs {
        val.set_attrs(Some(a.clone()));
    }
    if naflag {
        i.warningcall(call, format!("NaNs produced in function \"{}\"", name))?;
    }
    Ok(val)
}

math_fn!(do_floor, "floor", f64::floor);
math_fn!(do_ceiling, "ceiling", f64::ceil);
math_fn!(do_sqrt, "sqrt", f64::sqrt);
math_fn!(do_sign, "sign", |x: f64| if x > 0.0 {
    1.0
} else if x < 0.0 {
    -1.0
} else {
    0.0
});
math_fn!(do_exp, "exp", f64::exp);
math_fn!(do_expm1, "expm1", f64::exp_m1);
math_fn!(do_log1p, "log1p", f64::ln_1p);
math_fn!(do_cos, "cos", f64::cos);
math_fn!(do_sin, "sin", f64::sin);
math_fn!(do_tan, "tan", f64::tan);
math_fn!(do_acos, "acos", f64::acos);
math_fn!(do_asin, "asin", f64::asin);
math_fn!(do_atan, "atan", f64::atan);
math_fn!(do_cosh, "cosh", f64::cosh);
math_fn!(do_sinh, "sinh", f64::sinh);
math_fn!(do_tanh, "tanh", f64::tanh);
math_fn!(do_acosh, "acosh", f64::acosh);
math_fn!(do_asinh, "asinh", f64::asinh);
math_fn!(do_atanh, "atanh", f64::atanh);
math_fn!(do_cospi, "cospi", cospi);
math_fn!(do_sinpi, "sinpi", sinpi);
math_fn!(do_tanpi, "tanpi", tanpi);
// gamma family: nmath port (builtins/math.rs emits nmath's warnings)
fn do_gamma(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    super::math::do_math1_nmath(i, c, a, e, "gamma", crate::nmath::gammafn)
}
fn do_lgamma(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    super::math::do_math1_nmath(i, c, a, e, "lgamma", crate::nmath::lgammafn)
}
fn do_digamma(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    super::math::do_math1_nmath(i, c, a, e, "digamma", crate::nmath::digamma)
}
fn do_trigamma(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    super::math::do_math1_nmath(i, c, a, e, "trigamma", crate::nmath::trigamma)
}
// R: arithmetic.c:do_log1arg — log2/log10 route through the log family
// (math2 with base 2/10, z_logbase for complex).
fn do_log1arg(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env, base: f64, name: &str) -> RResult<Value> {
    if args.len() != 1 {
        return Err(Flow::error_call(
            call.clone(),
            format!(
                "{} argument{} passed to '{}' which requires 1",
                args.len(),
                if args.len() == 1 { "" } else { "s" },
                name
            ),
        ));
    }
    let x = args.pop().unwrap().value;
    let b = Value::real(base);
    if x.is_object() {
        // DispatchGroup("Math") on log2/log10 itself first, then on the
        // rewritten `log(x, base)` call.
        let dargs = vec![Arg::new(None, x.clone())];
        if let Some(r) = i.try_dispatch_group("Math", name, call, &dargs, env)? {
            return Ok(r);
        }
        let dargs = vec![Arg::new(None, x.clone()), Arg::new(None, b.clone())];
        if let Some(r) = i.try_dispatch_group("Math", "log", call, &dargs, env)? {
            return Ok(r);
        }
    }
    if matches!(x, Value::Cplx(_)) {
        return log_complex_math2(i, call, &x, &b, name);
    }
    log_math2(i, call, &x, &b)
}

fn do_log2(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_log1arg(i, c, a, e, 2.0, "log2")
}
fn do_log10(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    do_log1arg(i, c, a, e, 10.0, "log10")
}

fn cospi(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    // R: nmath/cospi.c uses C fmod
    let x = x.abs() % 2.0;
    if x % 1.0 == 0.5 {
        return 0.0;
    }
    if x == 1.0 {
        return -1.0;
    }
    if x == 0.0 {
        return 1.0;
    }
    (std::f64::consts::PI * x).cos()
}

fn sinpi(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let mut x = x % 2.0;
    if x <= -1.0 {
        x += 2.0;
    } else if x > 1.0 {
        x -= 2.0;
    }
    if x == 0.0 || x == 1.0 {
        return 0.0;
    }
    if x == 0.5 {
        return 1.0;
    }
    if x == -0.5 {
        return -1.0;
    }
    (std::f64::consts::PI * x).sin()
}

fn tanpi(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let mut x = x % 1.0;
    if x <= -0.5 {
        x += 1.0;
    } else if x > 0.5 {
        x -= 1.0;
    }
    if x == 0.0 {
        0.0
    } else if x == 0.5 {
        f64::NAN
    } else if x == 0.25 {
        1.0
    } else if x == -0.25 {
        -1.0
    } else {
        (std::f64::consts::PI * x).tan()
    }
}

/// The nmath port (kept reachable under the old paths).
pub use crate::nmath::{gammafn, lgammafn};

fn do_abs(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() != 1 {
        return Err(Flow::error_call(
            call.clone(),
            format!("{} arguments passed to 'abs' which requires 1", args.len()),
        ));
    }
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch_group("Math", "abs", call, &args, env)? {
            return Ok(r);
        }
    }
    match &args[0].value {
        Value::Int(v) | Value::Lgl(v) => {
            let mut out = Value::ints(v.data.iter().map(|&e| if e == NA_INTEGER { e } else { e.abs() }).collect());
            if let Some(a) = &v.attrs {
                out.set_attrs(Some(a.clone()));
            }
            Ok(out)
        }
        Value::Cplx(v) => {
            let mut out = Value::reals(v.data.iter().map(|z| z.re.hypot(z.im)).collect());
            if let Some(a) = &v.attrs {
                out.set_attrs(Some(a.clone()));
            }
            Ok(out)
        }
        _ => math1(i, call, args, env, "abs", f64::abs),
    }
}

fn do_trunc(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.is_empty() {
        return Err(Flow::error_call(call.clone(), "0 arguments passed to 'trunc' which requires 1"));
    }
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch_group("Math", "trunc", call, &args, env)? {
            return Ok(r);
        }
    }
    math1(i, call, vec![args[0].clone()], env, "trunc", f64::trunc)
}

// R: arithmetic.h:R_log — log with R's chosen values for x <= 0.
fn r_log(x: f64) -> f64 {
    if x > 0.0 {
        x.ln()
    } else if x == 0.0 {
        f64::NEG_INFINITY
    } else {
        f64::NAN
    }
}

// R: arithmetic.h:logbase
fn logbase(x: f64, base: f64) -> f64 {
    if base == 10.0 {
        if x > 0.0 {
            x.log10()
        } else if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    } else if base == 2.0 {
        if x > 0.0 {
            x.log2()
        } else if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    } else {
        r_log(x) / r_log(base)
    }
}

/// R's `isNumeric`: logical, integer (not factor) or double.
fn r_is_numeric(v: &Value) -> bool {
    match v {
        Value::Int(_) => !v.inherits("factor"),
        Value::Lgl(_) | Value::Real(_) => true,
        _ => false,
    }
}

/// The math2 kernel of `log(x, base)` on doubles
/// (R: arithmetic.c:math2 with `logbase`).
fn log_math2(i: &mut Interp, call: &Value, sa: &Value, sb: &Value) -> RResult<Value> {
    // SETUP_Math2
    if !r_is_numeric(sa) || !r_is_numeric(sb) {
        return Err(Flow::error_call(call.clone(), "non-numeric argument to mathematical function"));
    }
    let (na, nb) = (sa.len(), sb.len());
    if na == 0 || nb == 0 {
        // for 0-length a we want the attributes of a, as no recycling
        // will occur
        let mut sy = Value::reals(vec![]);
        if na == 0 {
            if let Some(a) = sa.attrs() {
                sy.set_attrs(Some(Box::new(a.clone())));
            }
        }
        return Ok(sy);
    }
    let n = na.max(nb);
    let a = real_view(sa);
    let b = real_view(sb);
    let mut naflag = false;
    let out: Vec<f64> = (0..n)
        .map(|k| {
            let (ai, bi) = (a[k % na], b[k % nb]);
            // if_NA_Math2_set
            if is_na_real(ai) || is_na_real(bi) {
                NA_REAL
            } else if ai.is_nan() || bi.is_nan() {
                f64::NAN
            } else {
                let y = logbase(ai, bi);
                if y.is_nan() {
                    naflag = true;
                }
                y
            }
        })
        .collect();
    let mut sy = Value::reals(out);
    // FINISH_Math2
    if naflag {
        i.warning("NaNs produced")?;
    }
    if n == na {
        if let Some(a) = sa.attrs() {
            sy.set_attrs(Some(Box::new(a.clone())));
        }
    } else if n == nb {
        if let Some(b) = sb.attrs() {
            sy.set_attrs(Some(Box::new(b.clone())));
        }
    }
    Ok(sy)
}

/// `z_logbase` over recycled operands (R: complex.c:complex_math2 for
/// the log family). The warning is a plain `warning()` like R's.
fn log_complex_math2(i: &mut Interp, call: &Value, sx: &Value, sbase: &Value, name: &str) -> RResult<Value> {
    let _ = call;
    let mut w = CoerceWarnings::default();
    let a = crate::coerce::to_cplx(sx, &mut w);
    let b = crate::coerce::to_cplx(sbase, &mut w);
    w.emit(i)?;
    let (na, nb) = (a.len(), b.len());
    if na == 0 || nb == 0 {
        return Ok(Value::cplxs(vec![]));
    }
    let n = na.max(nb);
    let mut naflag = false;
    let out: Vec<Complex> = (0..n)
        .map(|k| {
            let ai = a[k % na];
            let bi = b[k % nb];
            if is_na_real(ai.re) && is_na_real(ai.im) && is_na_real(bi.re) && is_na_real(bi.im) {
                Complex::NA
            } else {
                // z_logbase: clog(z)/clog(base)
                let r = super::complex::cdiv(super::complex::clog(ai), super::complex::clog(bi));
                if (r.re.is_nan() || r.im.is_nan())
                    && !(ai.re.is_nan() || ai.im.is_nan() || bi.re.is_nan() || bi.im.is_nan())
                {
                    naflag = true;
                }
                r
            }
        })
        .collect();
    let mut sy = Value::cplxs(out);
    if naflag {
        i.warning(format!("NaNs produced in function \"{}\"", name))?;
    }
    if n == na {
        if let Some(a) = sx.attrs() {
            sy.set_attrs(Some(Box::new(a.clone())));
        }
    } else if n == nb {
        if let Some(b) = sbase.attrs() {
            sy.set_attrs(Some(Box::new(b.clone())));
        }
    }
    Ok(sy)
}

/// `log(x, base)`: a special so that `log(x, base)` can dispatch on x
/// before base is evaluated.
// R: arithmetic.c:do_log / do_log_builtin
fn do_log(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    // evalListKeepMissing: evaluate the arguments, expanding `...` and
    // keeping the missing markers (an explicit `x=` stays missing).
    let ev = super::subset::eval_keep_missing(i, &args, env)?;
    let is_tag = |a: &Arg, names: &[&str]| a.tag.is_none_or(|t| names.contains(&&*t.name()));
    if ev.len() == 1 && is_tag(&ev[0], &["x"]) {
        let x = &ev[0].value;
        if !x.is_missing_arg() && !x.is_object() {
            if let Value::Cplx(_) = x {
                return complex_math1(i, call, x, "log");
            }
            return math1(i, call, vec![ev[0].clone()], env, "log", r_log);
        }
    } else if ev.len() == 2 && is_tag(&ev[0], &["x"]) && is_tag(&ev[1], &["base"]) {
        let (x, y) = (&ev[0].value, &ev[1].value);
        if !x.is_missing_arg() && !y.is_missing_arg() && !x.is_object() && !y.is_object() {
            if matches!(x, Value::Cplx(_)) || matches!(y, Value::Cplx(_)) {
                return log_complex_math2(i, call, x, y, "log");
            }
            return log_math2(i, call, x, y);
        }
    }
    // General path: match the argument names like R's do_log (the
    // matched call has exactly the formals x and base).
    let missing_x_err = |call: &Value| Flow::error_call(call.clone(), "argument \"x\" is missing, with no default");
    if ev.is_empty() {
        return Err(missing_x_err(call));
    }
    if ev.len() == 1 {
        // R's n == 1 branch never calls matchArgs_NR: the tag must be
        // missing or exactly "x", and the value present; it dispatches
        // with the argument as given (no base)
        let a = &ev[0];
        if a.value.is_missing_arg() || a.tag.is_some_and(|t| &*t.name() != "x") {
            return Err(missing_x_err(call));
        }
        let x = a.value.clone();
        if x.is_object() {
            if let Some(r) = i.try_dispatch_group("Math", "log", call, &ev, env)? {
                return Ok(r);
            }
        }
        if let Value::Cplx(_) = x {
            return complex_math1(i, call, &x, "log");
        }
        return math1(i, call, ev, env, "log", r_log);
    }
    // matchArgs_NR: signals the first unused argument onward.
    let m = super::match_args(ev, &["x", "base"], call)?;
    let x = m.vals[0].clone().unwrap_or_else(Value::missing_arg);
    if x.is_missing_arg() {
        return Err(missing_x_err(call));
    }
    let base = m.vals[1].clone().unwrap_or_else(|| Value::real(std::f64::consts::E));
    if base.is_object() || x.is_object() {
        // DispatchGroup("Math", ...) with the matched args.
        let dargs = vec![Arg::new(Some(s::X), x.clone()), Arg::new(Some(Sym::new("base")), base.clone())];
        if let Some(r) = i.try_dispatch_group("Math", "log", call, &dargs, env)? {
            return Ok(r);
        }
    }
    if base.len() == 0 {
        return Err(Flow::error_call(call.clone(), "invalid argument 'base' of length 0"));
    }
    if matches!(x, Value::Cplx(_)) || matches!(base, Value::Cplx(_)) {
        return log_complex_math2(i, call, &x, &base, "log");
    }
    log_math2(i, call, &x, &base)
}

#[allow(dead_code)]
fn unused(a: &[Arg]) -> Value {
    arg(a, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Double arithmetic is bit-identical with GNU R 4.5.0 itself (Debian
    /// r-base-core 4.5.0-3) in every recycling shape, NaN payloads (NA vs
    /// NaN) included: every pair of 16 special values (signalling and quiet
    /// NA, NaNs with several payloads, +-Inf, +-0, finite, huge, subnormal)
    /// for `+ - * / ^ %% %/%`, and NA_integer_/3L with doubles for
    /// `+ - * /`, recorded by `scripts/oracle-real-arith.R`.
    #[test]
    fn gnu_r_oracle() {
        use std::io::Read;
        let raw = include_bytes!("arith_r_oracle.txt.gz");
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s).unwrap();
        let hex = |h: &str| f64::from_bits(u64::from_str_radix(h, 16).unwrap());
        let op_of = |o: &str| match o {
            "add" | "+" => ArithOp::Plus,
            "sub" | "-" => ArithOp::Minus,
            "mul" | "*" => ArithOp::Times,
            "div" | "/" => ArithOp::Div,
            "pow" => ArithOp::Pow,
            "mod" => ArithOp::Mod,
            "idiv" => ArithOp::IDiv,
            other => panic!("bad op {other}"),
        };
        // (x, y, index of the checked element) for each shape
        let shape = |sh: &str, x: f64, y: f64| -> (Vec<f64>, Vec<f64>, usize) {
            match sh {
                "ss" => (vec![x], vec![y], 0),
                "vv" => (vec![x, x], vec![y, y], 1),
                "sv" => (vec![x], vec![y, y, y], 1),
                "vs" => (vec![x, x, x], vec![y], 1),
                "rc" => (vec![x; 4], vec![y, y], 2),
                other => panic!("bad shape {other}"),
            }
        };
        let mut checked = 0;
        for line in s.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            let (op, x, y, want, int_operand, sh) = if !matches!(t[0], "+" | "-" | "*" | "/") {
                // double with double: op shape x y result
                (op_of(t[0]), hex(t[2]), hex(t[3]), hex(t[4]), false, t[1].to_string())
            } else {
                // integer with double: op int double shape_order result
                let i = if t[1] == "NAi" { int_to_real(NA_INTEGER) } else { 3.0 };
                let (sh, order) = t[3].split_once('_').unwrap();
                let (x, y) = if order == "id" { (i, hex(t[2])) } else { (hex(t[2]), i) };
                (op_of(t[0]), x, y, hex(t[4]), true, sh.to_string())
            };
            let (a, b, k) = shape(&sh, x, y);
            let n = a.len().max(b.len());
            let got = real_binary(op, &a, &b, n, int_operand)[k];
            assert_eq!(got.to_bits(), want.to_bits(), "{line}");
            checked += 1;
        }
        assert!(checked > 9000, "{checked}");
    }
}
