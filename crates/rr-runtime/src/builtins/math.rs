//! Mathematical functions backed by the nmath port: the Math2 group
//! primitives `round`/`signif` (arithmetic.c `do_Math2`), the
//! `.Internal` functions of two, three and four numeric arguments
//! (`do_math2`, `do_math3`, `do_math4`: `beta`, `choose`, `atan2`, the
//! d/p/q distribution functions, ...), and the nmath-backed members of
//! the Math group (`gamma`, `lgamma`, `digamma`, `trigamma`).

use super::{internal, special};
use crate::coerce::{as_integer, CoerceWarnings};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::nmath;
use crate::nmath::binom as nb;
use crate::nmath::dist as nd;
use crate::nmath::norm as nn;
use crate::nmath::pgamma as npg;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    special("round", do_round, -1),
    special("signif", do_signif, -1),
    internal("atan2", do_atan2, 2),
    internal("lbeta", do_lbeta, 2),
    internal("beta", do_beta, 2),
    internal("lchoose", do_lchoose, 2),
    internal("choose", do_choose, 2),
    internal("psigamma", do_psigamma, 2),
    internal("dchisq", do_dchisq, 3),
    internal("pchisq", do_pchisq, 4),
    internal("qchisq", do_qchisq, 4),
    internal("dexp", do_dexp, 3),
    internal("pexp", do_pexp, 4),
    internal("qexp", do_qexp, 4),
    internal("dgeom", do_dgeom, 3),
    internal("pgeom", do_pgeom, 4),
    internal("qgeom", do_qgeom, 4),
    internal("dpois", do_dpois, 3),
    internal("ppois", do_ppois, 4),
    internal("qpois", do_qpois, 4),
    internal("dt", do_dt, 3),
    internal("pt", do_pt, 4),
    internal("qt", do_qt, 4),
    internal("dbeta", do_dbeta, 4),
    internal("pbeta", do_pbeta, 5),
    internal("qbeta", do_qbeta, 5),
    internal("df", do_df, 4),
    internal("pf", do_pf, 5),
    internal("qf", do_qf, 5),
    internal("dbinom", do_dbinom, 4),
    internal("pbinom", do_pbinom, 5),
    internal("qbinom", do_qbinom, 5),
    internal("dcauchy", do_dcauchy, 4),
    internal("pcauchy", do_pcauchy, 5),
    internal("qcauchy", do_qcauchy, 5),
    internal("dgamma", do_dgamma, 4),
    internal("pgamma", do_pgamma, 5),
    internal("qgamma", do_qgamma, 5),
    internal("dlnorm", do_dlnorm, 4),
    internal("plnorm", do_plnorm, 5),
    internal("qlnorm", do_qlnorm, 5),
    internal("dlogis", do_dlogis, 4),
    internal("plogis", do_plogis, 5),
    internal("qlogis", do_qlogis, 5),
    internal("dnorm", do_dnorm, 4),
    internal("pnorm", do_pnorm, 5),
    internal("qnorm", do_qnorm, 5),
    internal("dunif", do_dunif, 4),
    internal("punif", do_punif, 5),
    internal("qunif", do_qunif, 5),
    internal("dweibull", do_dweibull, 4),
    internal("pweibull", do_pweibull, 5),
    internal("qweibull", do_qweibull, 5),
];

// ----------------------------------------------------------------------
// Shared helpers

/// R's `isNumeric`: logical, integer (not factor) or double.
fn is_numeric(v: &Value) -> bool {
    match v {
        Value::Int(_) => !v.inherits("factor"),
        Value::Lgl(_) | Value::Real(_) => true,
        _ => false,
    }
}

const MSG_NONNUM_MATH: &str = "non-numeric argument to mathematical function";

/// `errorcall(lcall, ...)` where an `.Internal` passes R_CurrentExpression
/// (behave like `error()`).
fn error_at(call: Option<&Value>, msg: &str) -> Flow {
    match call {
        Some(c) => Flow::error_call(c.clone(), msg),
        None => Flow::error(msg),
    }
}

fn reals(v: &Value) -> Vec<f64> {
    crate::coerce::to_real(v, &mut CoerceWarnings::default())
}

/// Emit the warnings nmath raised (MATHLIB_WARNING is R's `warning()`,
/// which drops one trailing newline from the message).
pub fn emit_nmath_warnings(i: &mut Interp) -> RResult<()> {
    for w in nmath::take_warnings() {
        let w = w.strip_suffix('\n').map(str::to_string).unwrap_or(w);
        i.warning(w)?;
    }
    Ok(())
}

/// `if_NA_Math2_set` and friends: NA if any input is NA, NaN if any NaN.
fn na_result(xs: &[f64]) -> Option<f64> {
    if xs.iter().any(|&x| is_na_real(x)) {
        Some(NA_REAL)
    } else if xs.iter().any(|x| x.is_nan()) {
        Some(f64::NAN)
    } else {
        None
    }
}

fn copy_all_attrs(from: &Value, to: &mut Value) {
    if let Some(a) = from.attrs() {
        to.set_attrs(Some(Box::new(a.clone())));
    }
}

/// R: arithmetic.c:math2 (and math2_1/math2_2: `f` closes over the int
/// flags).
fn math2(
    i: &mut Interp,
    call: Option<&Value>,
    sa: &Value,
    sb: &Value,
    f: &dyn Fn(f64, f64) -> f64,
) -> RResult<Value> {
    if !is_numeric(sa) || !is_numeric(sb) {
        return Err(error_at(call, MSG_NONNUM_MATH));
    }
    let (na, nb) = (sa.len(), sb.len());
    if na == 0 || nb == 0 {
        let mut sy = Value::reals(vec![]);
        if na == 0 {
            copy_all_attrs(sa, &mut sy);
        }
        return Ok(sy);
    }
    let n = na.max(nb);
    let a = reals(sa);
    let b = reals(sb);
    nmath::take_warnings();
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for k in 0..n {
        let (ai, bi) = (a[k % na], b[k % nb]);
        let v = match na_result(&[ai, bi]) {
            Some(v) => v,
            None => {
                let v = f(ai, bi);
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
        i.warning("NaNs produced")?;
    }
    let mut sy = Value::reals(y);
    if n == na {
        copy_all_attrs(sa, &mut sy);
    } else if n == nb {
        copy_all_attrs(sb, &mut sy);
    }
    Ok(sy)
}

/// R: arithmetic.c:math3_1 / math3_2 (`error()` for non-numeric args)
fn math3(i: &mut Interp, sa: &Value, sb: &Value, sc: &Value, f: &dyn Fn(f64, f64, f64) -> f64) -> RResult<Value> {
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
    let a = reals(sa);
    let b = reals(sb);
    let c = reals(sc);
    nmath::take_warnings();
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for k in 0..n {
        let (ai, bi, ci) = (a[k % na], b[k % nb], c[k % nc]);
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
        i.warning("NaNs produced")?;
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

fn arity(args: &[Arg], n: usize, name: &str, _call: &Value) -> RResult<()> {
    super::random::check_arity_internal(args, n, name)
}

fn flag(v: &Value) -> bool {
    // asInteger(): NA becomes NA_INTEGER, which C treats as true
    as_integer(v) != 0
}

// ----------------------------------------------------------------------
// round / signif (do_Math2)

/// R's `evalListKeepMissing`: evaluate arguments, expanding `...`, and
/// keep empty arguments as the missing-argument marker.
fn eval_list_keep_missing(i: &mut Interp, args: &[Arg], env: &Env) -> RResult<Vec<Arg>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        match &a.value {
            Value::Sym(sym) if *sym == s::DOTS => match env.lookup(s::DOTS) {
                Some((Value::Dots(d), _)) => {
                    for item in d.iter() {
                        if item.value.is_missing_arg() {
                            out.push(item.clone());
                        } else {
                            let v = i.eval(&item.value, env)?;
                            out.push(Arg::new(item.tag, v));
                        }
                    }
                }
                Some((v, _)) if v.is_missing_arg() || v.is_null() => {}
                _ => return Err(Flow::error("'...' used in an incorrect context")),
            },
            v if v.is_missing_arg() => out.push(a.clone()),
            v => {
                let v = i.eval(v, env)?;
                out.push(Arg::new(a.tag, v));
            }
        }
    }
    Ok(out)
}

fn x_sym() -> Sym {
    Sym::new("x")
}

/// R: arithmetic.c:match_round_gen_args -- formals (x, ...); `x` first.
fn match_round_gen_args(args: Vec<Arg>, call: &Value) -> RResult<Vec<Arg>> {
    if !args.is_empty() && args[0].tag.is_none() && args.get(1).is_none_or(|a| a.tag != Some(x_sym())) && args.len() <= 2
    {
        return Ok(args);
    }
    let m = super::match_args(args, &["x", "..."], call)?;
    let mut out = vec![Arg::pos(m.vals[0].clone().unwrap_or_else(Value::missing_arg))];
    out.extend(m.dots);
    Ok(out)
}

/// R: arithmetic.c:match_Math2_dflt_args -- formals (x, digits).
fn match_math2_dflt_args(args: Vec<Arg>, call: &Value) -> RResult<(Value, Value)> {
    if args.is_empty() {
        return Ok((Value::missing_arg(), Value::missing_arg()));
    }
    if args.len() == 1 && args[0].tag.is_none() {
        return Ok((args[0].value.clone(), Value::missing_arg()));
    }
    if args.len() == 2 && args[0].tag.is_none() && args[1].tag.is_none() {
        return Ok((args[0].value.clone(), args[1].value.clone()));
    }
    let m = super::match_args(args, &["x", "digits"], call)?;
    Ok((
        m.vals[0].clone().unwrap_or_else(Value::missing_arg),
        m.vals[1].clone().unwrap_or_else(Value::missing_arg),
    ))
}

/// R: arithmetic.c:do_Math2 (round = 10001, signif = 10004)
fn do_math2_group(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, is_signif: bool) -> RResult<Value> {
    let name = if is_signif { "signif" } else { "round" };
    let dflt_digits = if is_signif { 6.0 } else { 0.0 };
    let args = eval_list_keep_missing(i, &args, env)?;
    let args: Vec<Arg> = if is_signif {
        let (x, mut d) = match_math2_dflt_args(args, call)?;
        if d.is_missing_arg() {
            d = Value::real(dflt_digits);
        }
        vec![Arg::pos(x), Arg::pos(d)]
    } else {
        match_round_gen_args(args, call)?
    };
    // DispatchGroup("Math", call2, ...) with call2 = (round <evaluated args>)
    if args.first().is_some_and(|a| a.value.is_object()) {
        let fun = match call {
            Value::Lang(l) => l.fun.clone(),
            _ => Value::sym(name),
        };
        let call2 = Value::lang(fun, args.clone());
        if let Some(r) = i.try_dispatch_group("Math", name, &call2, &args, env)? {
            return Ok(r);
        }
    }
    let (x, digits) = if is_signif {
        (args[0].value.clone(), args[1].value.clone())
    } else {
        let (x, mut d) = match_math2_dflt_args(args, call)?;
        if d.is_missing_arg() {
            d = Value::real(dflt_digits);
        }
        (x, d)
    };
    if x.is_missing_arg() {
        return Err(Flow::error_call(call.clone(), "argument \"x\" is missing, with no default"));
    }
    if digits.is_empty() {
        return Err(Flow::error_call(call.clone(), "invalid second argument of length 0"));
    }
    if matches!(x, Value::Cplx(_)) {
        return complex_round(i, &x, &digits, is_signif);
    }
    let f: fn(f64, f64) -> f64 = if is_signif { nmath::fround::fprec } else { nmath::fround::fround };
    math2(i, Some(call), &x, &digits, &f)
}

fn do_round(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_math2_group(i, call, args, env, false)
}

fn do_signif(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_math2_group(i, call, args, env, true)
}

/// R: complex.c:z_prec_r -- signif() for complex numbers
fn z_prec_r(x: Complex, digits: f64) -> Complex {
    const MAX_DIGITS: i32 = 22;
    let mut r = x;
    let mut m = 0.0;
    let m1 = x.re.abs();
    let m2 = x.im.abs();
    if m1.is_finite() {
        m = m1;
    }
    if m2.is_finite() && m2 > m {
        m = m2;
    }
    if m == 0.0 {
        return r;
    }
    if !digits.is_finite() {
        if digits > 0.0 {
            return r;
        }
        r.re = 0.0;
        r.im = 0.0;
        return r;
    }
    let mut dig = (digits + 0.5).floor() as i32;
    if dig > MAX_DIGITS {
        return r;
    } else if dig < 1 {
        dig = 1;
    }
    let mag = m.log10().floor() as i32;
    dig = dig - mag - 1;
    if dig > 306 {
        let pow10 = 1.0e4;
        let d = (dig - 4) as f64;
        r.re = nmath::fround::fround(pow10 * x.re, d) / pow10;
        r.im = nmath::fround::fround(pow10 * x.im, d) / pow10;
    } else {
        let d = dig as f64;
        r.re = nmath::fround::fround(x.re, d);
        r.im = nmath::fround::fround(x.im, d);
    }
    r
}

fn complex_round(i: &mut Interp, x: &Value, digits: &Value, is_signif: bool) -> RResult<Value> {
    if is_signif {
        complex_math2(i, "signif", x, digits, &|a, b| z_prec_r(a, b.re))
    } else {
        // R: complex.c:z_rround
        complex_math2(i, "round", x, digits, &|a, b| {
            Complex::new(nmath::fround::fround(a.re, b.re), nmath::fround::fround(a.im, b.re))
        })
    }
}

/// C99 `double complex` as passed by value on the SysV ABI.
#[repr(C)]
#[derive(Clone, Copy)]
struct CDouble {
    re: f64,
    im: f64,
}

unsafe extern "C" {
    fn catan(z: CDouble) -> CDouble;
    /// libgcc's complex division, which C's `/` on `double complex` uses
    fn __divdc3(a: f64, b: f64, c: f64, d: f64) -> CDouble;
}

/// R: complex.c:z_atan2
fn z_atan2(csn: Complex, ccs: Complex) -> Complex {
    let dr;
    if ccs.re == 0.0 && ccs.im == 0.0 {
        if csn.re == 0.0 && csn.im == 0.0 {
            return Complex::new(NA_REAL, NA_REAL);
        }
        let y = csn.re;
        dr = Complex::new(if y.is_nan() { y } else if y >= 0.0 { nmath::M_PI_2 } else { -nmath::M_PI_2 }, 0.0);
    } else {
        let q = unsafe { __divdc3(csn.re, csn.im, ccs.re, ccs.im) };
        let t = unsafe { catan(q) };
        let mut d = Complex::new(t.re, t.im);
        if ccs.re < 0.0 {
            d.re += nmath::M_PI;
        }
        if d.re > nmath::M_PI {
            d.re -= 2.0 * nmath::M_PI;
        }
        dr = d;
    }
    dr
}

/// R: complex.c:complex_math2
fn complex_math2(
    i: &mut Interp,
    name: &str,
    x: &Value,
    digits: &Value,
    f: &dyn Fn(Complex, Complex) -> Complex,
) -> RResult<Value> {
    let mut w = CoerceWarnings::default();
    let a = crate::coerce::to_cplx(x, &mut w);
    let b = crate::coerce::to_cplx(digits, &mut w);
    w.emit(i)?;
    let (na, nb) = (a.len(), b.len());
    if na == 0 || nb == 0 {
        return Ok(Value::cplxs(vec![]));
    }
    let n = na.max(nb);
    let mut naflag = false;
    let mut y = Vec::with_capacity(n);
    for k in 0..n {
        let (ai, bi) = (a[k % na], b[k % nb]);
        let v = if is_na_real(ai.re) && is_na_real(ai.im) && is_na_real(bi.re) && is_na_real(bi.im) {
            Complex::NA
        } else {
            let v = f(ai, bi);
            if (v.re.is_nan() || v.im.is_nan())
                && !(ai.re.is_nan() || ai.im.is_nan() || bi.re.is_nan() || bi.im.is_nan())
            {
                naflag = true;
            }
            v
        };
        y.push(v);
    }
    if naflag {
        i.warning(format!("NaNs produced in function \"{}\"", name))?;
    }
    let mut sy = Value::cplxs(y);
    if n == na {
        copy_all_attrs(x, &mut sy);
    } else if n == nb {
        copy_all_attrs(digits, &mut sy);
    }
    Ok(sy)
}

// ----------------------------------------------------------------------
// Math group members backed by nmath (called from arith.rs)

/// R: arithmetic.c:do_math1 + math1 for the nmath-backed functions
/// (gamma, lgamma, digamma, trigamma), emitting nmath's warnings.
pub fn do_math1_nmath(
    i: &mut Interp,
    call: &Value,
    args: Vec<Arg>,
    env: &Env,
    name: &str,
    f: fn(f64) -> f64,
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
        return Err(Flow::error_call(call.clone(), "unimplemented complex function"));
    }
    if !is_numeric(x) {
        return Err(Flow::error_call(call.clone(), MSG_NONNUM_MATH));
    }
    let a = reals(x);
    nmath::take_warnings();
    let mut naflag = false;
    let y: Vec<f64> = a
        .iter()
        .map(|&v| {
            let r = f(v);
            if r.is_nan() {
                if v.is_nan() {
                    return v;
                }
                naflag = true;
            }
            r
        })
        .collect();
    emit_nmath_warnings(i)?;
    if naflag {
        i.warningcall(call, "NaNs produced")?;
    }
    let mut val = Value::reals(y);
    copy_all_attrs(x, &mut val);
    Ok(val)
}

// ----------------------------------------------------------------------
// .Internal math2 functions

macro_rules! m2 {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
            arity(&args, 2, $rname, call)?;
            math2(i, None, &args[0].value, &args[1].value, &$f)
        }
    };
}

macro_rules! m2_1 {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
            arity(&args, 3, $rname, call)?;
            let lg = flag(&args[2].value);
            math2(i, None, &args[0].value, &args[1].value, &|a, b| $f(a, b, lg))
        }
    };
}

macro_rules! m2_2 {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
            arity(&args, 4, $rname, call)?;
            let lt = flag(&args[2].value);
            let lg = flag(&args[3].value);
            math2(i, None, &args[0].value, &args[1].value, &|a, b| $f(a, b, lt, lg))
        }
    };
}

macro_rules! m3_1 {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
            arity(&args, 4, $rname, call)?;
            let lg = flag(&args[3].value);
            math3(i, &args[0].value, &args[1].value, &args[2].value, &|a, b, c| $f(a, b, c, lg))
        }
    };
}

macro_rules! m3_2 {
    ($fname:ident, $rname:expr, $f:expr) => {
        fn $fname(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
            arity(&args, 5, $rname, call)?;
            let lt = flag(&args[3].value);
            let lg = flag(&args[4].value);
            math3(i, &args[0].value, &args[1].value, &args[2].value, &|a, b, c| $f(a, b, c, lt, lg))
        }
    };
}

fn do_atan2(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    arity(&args, 2, "atan2", call)?;
    if matches!(args[0].value, Value::Cplx(_)) || matches!(args[1].value, Value::Cplx(_)) {
        return complex_math2(i, "atan2", &args[0].value, &args[1].value, &z_atan2);
    }
    math2(i, None, &args[0].value, &args[1].value, &f64::atan2)
}

m2!(do_lbeta, "lbeta", nmath::lbeta);
m2!(do_beta, "beta", nmath::beta_fn);
m2!(do_lchoose, "lchoose", nmath::lchoose);
m2!(do_choose, "choose", nmath::choose);
m2!(do_psigamma, "psigamma", nmath::psigamma);

m2_1!(do_dchisq, "dchisq", npg::dchisq);
m2_2!(do_pchisq, "pchisq", npg::pchisq);
m2_2!(do_qchisq, "qchisq", npg::qchisq);
m2_1!(do_dexp, "dexp", nd::dexp);
m2_2!(do_pexp, "pexp", nd::pexp);
m2_2!(do_qexp, "qexp", nd::qexp);
m2_1!(do_dgeom, "dgeom", nd::dgeom);
m2_2!(do_pgeom, "pgeom", nd::pgeom);
m2_2!(do_qgeom, "qgeom", nd::qgeom);
m2_1!(do_dpois, "dpois", npg::dpois);
m2_2!(do_ppois, "ppois", npg::ppois);
m2_2!(do_qpois, "qpois", npg::qpois);
m2_1!(do_dt, "dt", nmath::tdist::dt);
m2_2!(do_pt, "pt", nmath::tdist::pt);
m2_2!(do_qt, "qt", nmath::tdist::qt);

m3_1!(do_dbeta, "dbeta", nmath::beta::dbeta);
m3_2!(do_pbeta, "pbeta", nmath::beta::pbeta);
m3_2!(do_qbeta, "qbeta", nmath::beta::qbeta);
m3_1!(do_df, "df", nmath::fdist::df);
m3_2!(do_pf, "pf", nmath::fdist::pf);
m3_2!(do_qf, "qf", nmath::fdist::qf);

m3_1!(do_dbinom, "dbinom", nb::dbinom);
m3_2!(do_pbinom, "pbinom", nb::pbinom);
m3_2!(do_qbinom, "qbinom", nb::qbinom);
m3_1!(do_dcauchy, "dcauchy", nd::dcauchy);
m3_2!(do_pcauchy, "pcauchy", nd::pcauchy);
m3_2!(do_qcauchy, "qcauchy", nd::qcauchy);
m3_1!(do_dgamma, "dgamma", npg::dgamma);
m3_2!(do_pgamma, "pgamma", npg::pgamma);
m3_2!(do_qgamma, "qgamma", npg::qgamma);
m3_1!(do_dlnorm, "dlnorm", nd::dlnorm);
m3_2!(do_plnorm, "plnorm", nd::plnorm);
m3_2!(do_qlnorm, "qlnorm", nd::qlnorm);
m3_1!(do_dlogis, "dlogis", nd::dlogis);
m3_2!(do_plogis, "plogis", nd::plogis);
m3_2!(do_qlogis, "qlogis", nd::qlogis);
m3_1!(do_dnorm, "dnorm", nn::dnorm);
m3_2!(do_pnorm, "pnorm", nn::pnorm);
m3_2!(do_qnorm, "qnorm", nn::qnorm);
m3_1!(do_dunif, "dunif", nd::dunif);
m3_2!(do_punif, "punif", nd::punif);
m3_2!(do_qunif, "qunif", nd::qunif);
m3_1!(do_dweibull, "dweibull", nd::dweibull);
m3_2!(do_pweibull, "pweibull", nd::pweibull);
m3_2!(do_qweibull, "qweibull", nd::qweibull);
