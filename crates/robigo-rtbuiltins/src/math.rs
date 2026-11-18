//! arithmetic.c's mathematical functions: `do_math1`'s rows (`floor`,
//! `ceiling`, `sqrt`, `sign`, `exp` ... `tanpi`, with `math1` and
//! `math1_ari`), `trunc` (`do_trunc`), `abs` (`do_abs`), and `log`, `log2`
//! and `log10` (`do_log`, `do_log1arg`, with `math2`). All are "Math"
//! group generics.

use crate::attrib::Attrs;
use crate::registry::Builtin;
use crate::util::{is_numeric, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Cx, Diag, Dispatched, Generic, Out, RResult, Ref, Root, View};

const NONNUM: &str = "non-numeric argument to mathematical function";

/// `DispatchGroup("Math", ...)` for an object argument.
pub(crate) fn math_dispatch<'o>(
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

/// The double coercion of a numeric vector's elements (`coerceVector(sa,
/// REALSXP)`: an integer or logical NA is NA_real_).
fn reals(x: Ref<'_>) -> Option<Vec<f64>> {
    let int = |i: i32| if i == i32::MIN { robigo_text::NA_REAL } else { i as f64 };
    Some(match x.view() {
        View::Real(v) => v.as_slice().unwrap().to_vec(),
        View::Int(v) => v.as_slice().unwrap().iter().map(|&i| int(i)).collect(),
        View::Lgl(v) => v.as_slice().unwrap().iter().map(|i| int(i.0)).collect(),
        _ => return None,
    })
}

/// Raise, as `warning()`s, what nmath's `ML_WARNING`/`MATHLIB_WARNING`
/// queued.
// R: nmath/nmath.h:MATHLIB_WARNING
fn nmath_warnings(cx: &mut Cx) -> RResult<()> {
    for w in robigo_numerics::take_warnings() {
        cx.warning(w.strip_suffix('\n').unwrap_or(&w))?;
    }
    Ok(())
}

/// `math1(sa, f, lcall)` (and `math1_ari`, whose `x == arg` test is in
/// `f`): `f` over the double coercion of a numeric vector, its attributes
/// kept, "NaNs produced" against `lcall` when a NaN appears from a number.
/// `nmath`: `f` is nmath's, whose warnings come first.
// R: arithmetic.c:math1
fn math1(cx: &mut Cx, call: Root<'_>, x: Root<'_>, f: fn(f64) -> f64, nmath: bool, out: Out<'_>) -> RResult<()> {
    if !is_numeric(x.bind(cx)) {
        return Err(cx.error_call(Some(call), NONNUM));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "a math function of a package ALTREP vector"));
    }
    let naflag = cx.build_with(out, |cx| {
        if nmath {
            // a rerun after a collection computes (and warns) again
            robigo_numerics::take_warnings();
        }
        let xv = x.bind(cx);
        let mut a = Attrs::new();
        a.copy_all(xv);
        let Some(vals) = reals(xv) else { return Ok(Built::fail(Diag::error_call(Some(call), NONNUM))) };
        let mut naflag = false;
        let mut r = cx.alloc_with(vals.len(), |i| {
            let x = vals[i];
            // ISNAN(x) implies ISNAN(f(x)): x is tested only then
            let y = f(x);
            if y.is_nan() {
                if x.is_nan() {
                    // the incoming NaN (or NA) is kept
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
    if nmath {
        nmath_warnings(cx)?;
    }
    if naflag {
        cx.warning_call(Some(call), "NaNs produced")?;
    }
    Ok(())
}

/// `math2(sa, sb, f, lcall)`: `f` over the double coercions of two
/// numeric vectors, recycled (silently) to the longer; NA where either is
/// NA, else NaN where either is NaN. The longer's attributes are kept (the
/// first's on a tie; the first's alone if it is empty), and "NaNs
/// produced" is a `warning()`, against the context.
// R: arithmetic.c:math2
fn math2(cx: &mut Cx, call: Root<'_>, sa: Root<'_>, sb: Root<'_>, f: fn(f64, f64) -> f64, out: Out<'_>) -> RResult<()> {
    if !is_numeric(sa.bind(cx)) || !is_numeric(sb.bind(cx)) {
        return Err(cx.error_call(Some(call), NONNUM));
    }
    if sa.bind(cx).is_package_altrep() || sb.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "a math function of a package ALTREP vector"));
    }
    let naflag = cx.build_with(out, |cx| {
        let (av, bv) = (sa.bind(cx), sb.bind(cx));
        let (na, nb) = (av.len().unwrap_or(0), bv.len().unwrap_or(0));
        let mut attrs = Attrs::new();
        if na == 0 || nb == 0 {
            if na == 0 {
                attrs.copy_all(av);
            }
            let mut r = cx.alloc_with::<f64>(0, |_| 0.0)?;
            attrs.apply(cx, &mut r)?;
            return Ok(Built::done(r.finish()).with(false));
        }
        let n = na.max(nb);
        if n == na {
            attrs.copy_all(av);
        } else if n == nb {
            attrs.copy_all(bv);
        }
        let (Some(a), Some(b)) = (reals(av), reals(bv)) else {
            return Ok(Built::fail(Diag::error_call(Some(call), NONNUM)));
        };
        let mut naflag = false;
        let mut r = cx.alloc_with(n, |i| {
            let (ai, bi) = (a[i % na], b[i % nb]);
            if robigo_text::is_na(ai) || robigo_text::is_na(bi) {
                robigo_text::NA_REAL
            } else if ai.is_nan() || bi.is_nan() {
                f64::NAN
            } else {
                let y = f(ai, bi);
                naflag |= y.is_nan();
                y
            }
        })?;
        attrs.apply(cx, &mut r)?;
        Ok(Built::done(r.finish()).with(naflag))
    })?;
    if naflag {
        cx.warning("NaNs produced")?;
    }
    Ok(())
}

/// `R_log(x)`: -Inf at 0 and NaN below, whatever the C library does.
// R: arithmetic.h:R_log
/// bcEval's `math1funs` (eval.c): the function `MATH1`'s index names, the
/// C library's (or nmath's) as the table has it, never `do_math1`'s
/// wrappers, whose exact answers at `x == 0` drop `-0`; and whether it is
/// nmath's, which may queue a warning (`ML_WARNING`).
// R: eval.c:math1funs
pub const BC_MATH1: [(&str, fn(f64) -> f64, bool); 24] = [
    ("floor", f64::floor, false),
    ("ceiling", f64::ceil, false),
    ("sign", sign, false),
    ("expm1", robigo_numerics::expm1, false),
    ("log1p", robigo_numerics::log1p, false),
    ("cos", f64::cos, false),
    ("sin", f64::sin, false),
    ("tan", f64::tan, false),
    ("acos", f64::acos, false),
    ("asin", f64::asin, false),
    ("atan", f64::atan, false),
    ("cosh", f64::cosh, false),
    ("sinh", f64::sinh, false),
    ("tanh", f64::tanh, false),
    ("acosh", robigo_numerics::acosh, false),
    ("asinh", robigo_numerics::asinh, false),
    ("atanh", robigo_numerics::atanh, false),
    ("lgamma", robigo_numerics::lgammafn, true),
    ("gamma", robigo_numerics::gammafn, true),
    ("digamma", robigo_numerics::digamma, true),
    ("trigamma", robigo_numerics::trigamma, true),
    ("cospi", robigo_numerics::cospi, true),
    ("sinpi", robigo_numerics::sinpi, true),
    ("tanpi", robigo_numerics::r_tanpi, true),
];

/// The warnings nmath queued since the last take (`MATHLIB_WARNING`'s
/// messages, each ending in a newline).
pub fn take_nmath_warnings() -> Vec<String> {
    robigo_numerics::take_warnings()
}

/// `R_log(x)`, bcEval's `LOG` of one number.
// R: arithmetic.c:R_log
pub fn bc_log(x: f64) -> f64 {
    r_log(x)
}

/// `logbase(x, base)`, bcEval's `LOGBASE` of two numbers.
// R: arithmetic.c:logbase
pub fn bc_logbase(x: f64, base: f64) -> f64 {
    logbase(x, base)
}

fn r_log(x: f64) -> f64 {
    if x > 0.0 {
        x.ln()
    } else if x == 0.0 {
        f64::NEG_INFINITY
    } else {
        f64::NAN
    }
}

/// `logbase(x, base)`: `log10` and `log2` for those bases (HAVE_LOG10,
/// HAVE_LOG2), else `R_log(x) / R_log(base)`.
// R: arithmetic.h:logbase
fn logbase(x: f64, base: f64) -> f64 {
    let pole = |f: fn(f64) -> f64| {
        if x > 0.0 {
            f(x)
        } else if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        }
    };
    if base == 10.0 {
        return pole(f64::log10);
    }
    if base == 2.0 {
        return pole(f64::log2);
    }
    r_log(x) / r_log(base)
}

const LOG: Generic = Generic::group("Math", "log");

/// `log(x, base = exp(1))` on its evaluated arguments, missing ones kept
/// (robigo-rt's `log` special evaluates them, as `do_log` does):
/// `log(x)` and `log(x, b)` of non-objects skip the matching and the
/// dispatch.
// R: arithmetic.c:do_log_builtin
pub fn do_log_builtin(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let n = args.len();
        let missing = cx.missing_arg();
        let tag_is = |cx: &Cx, i: usize, name: &[u8]| args.name(cx, i).is_some_and(|t| t.name().bytes() == name);
        if n == 1 && args.name(cx, 0).is_none() {
            // log(x)
            let x = args.get(cx, 0);
            if x != missing && !x.is_object() {
                if x.sexptype() == CPLXSXP {
                    return Err(not_yet(cx, Some(call), "log() of a complex vector (complex_math1)"));
                }
                return math1(cx, call, args.root(0), r_log, false, out);
            }
        } else if n == 2 && args.name(cx, 0).is_none() && (args.name(cx, 1).is_none() || tag_is(cx, 1, b"base")) {
            // log(x, y) or log(x, base = y)
            let (x, y) = (args.get(cx, 0), args.get(cx, 1));
            if x != missing && y != missing && !x.is_object() && !y.is_object() {
                if x.sexptype() == CPLXSXP || y.sexptype() == CPLXSXP {
                    return Err(not_yet(cx, Some(call), "log() of a complex vector (complex_math2)"));
                }
                return math2(cx, call, args.root(0), args.root(1), logbase, out);
            }
        }
        if n == 1 {
            if args.get(cx, 0) == missing || (args.name(cx, 0).is_some() && !tag_is(cx, 0, b"x")) {
                return Err(cx.missing_arg_error("x", Some(call), "log1Error"));
            }
            let Dispatched::Default(out) = math_dispatch(cx, &LOG, call, args, env, out)? else { return Ok(()) };
            if args.get(cx, 0).sexptype() == CPLXSXP {
                return Err(not_yet(cx, Some(call), "log() of a complex vector (complex_math1)"));
            }
            return math1(cx, call, args.root(0), r_log, false, out);
        }
        // match the names if supplied: after it, the arguments are
        // (x, base), untagged
        let m = crate::util::match_args(cx, call, &["x", "base"], args)?;
        let missing = cx.missing_arg();
        let x = match m.formals[0] {
            Some(i) if args.get(cx, i) != missing => args.root(i),
            _ => return Err(cx.missing_arg_error("x", Some(call), "log2Error")),
        };
        let base = match m.formals[1] {
            Some(i) if args.get(cx, i) != cx.missing_arg() => args.root(i),
            // DFLT_LOG_BASE
            _ => s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[std::f64::consts::E])?.finish().into())))?,
        };
        let args = s.args(cx, &[x.bind(cx), base.bind(cx)], None);
        let Dispatched::Default(out) = math_dispatch(cx, &LOG, call, args, env, out)? else { return Ok(()) };
        let Some(len) = crate::builtin::length_mut(cx, base)? else {
            return Err(not_yet(cx, Some(call), "log() with an environment 'base' robigo cannot list"));
        };
        if len == 0 {
            return Err(cx.error_call(Some(call), "invalid argument 'base' of length 0"));
        }
        if x.bind(cx).sexptype() == CPLXSXP || base.bind(cx).sexptype() == CPLXSXP {
            return Err(not_yet(cx, Some(call), "log() of a complex vector (complex_math2)"));
        }
        math2(cx, call, x, base, logbase, out)
    })
}

/// `log2(x)` (code 10002) and `log10(x)` (code 10010): `logbase` with
/// the base 2 or 10.
// R: arithmetic.c:do_log1arg
pub fn do_log1arg<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    struct Log1<const CODE: i32>;
    impl<const CODE: i32> Log1<CODE> {
        const NAME: &'static str = match CODE {
            10002 => "log2",
            10010 => "log10",
            _ => panic!("robigo: not a do_log1arg code"),
        };
        const GENERIC: Generic = Generic::group("Math", Self::NAME);
        const ROW: &'static Builtin = crate::registry::find(Self::NAME, None);
    }
    Log1::<CODE>::ROW.check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = math_dispatch(cx, &Log1::<CODE>::GENERIC, call, args, env, out)? else {
        return Ok(());
    };
    let base = if CODE == 10010 { 10.0 } else { 2.0 };
    cx.scope(|cx, s| {
        let tmp = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[base])?.finish().into())))?;
        // dispatch again, on `log(x, base)` with (x, base): a call through
        // a `.default` name skips the first (DispatchGroup), not this one
        let call2 = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let log = cx.install(b"log")?;
                cx.lang3(log.as_ref(), args.get(cx, 0), tmp.bind(cx))
            })
        })?;
        let args2 = s.args(cx, &[args.get(cx, 0), tmp.bind(cx)], None);
        let Dispatched::Default(out) = math_dispatch(cx, &Log1::<CODE>::GENERIC, call2, args2, env, out)? else {
            return Ok(());
        };
        if args.get(cx, 0).sexptype() == CPLXSXP {
            let what = format!("{}() of a complex vector (complex_math2)", Log1::<CODE>::NAME);
            return Err(not_yet(cx, Some(call2), &what));
        }
        math2(cx, call, args.root(0), tmp, logbase, out)
    })
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

/// `f_x_x(x, f, m)`: `x` itself where `|x| <= m`, else `f(x)`.
// R: arithmetic.c:f_x_x
#[inline]
fn f_x_x(x: f64, f: fn(f64) -> f64, m: f64) -> f64 {
    if x.abs() <= m { x } else { f(x) }
}

/// `Rexp(x)`: `1 + x` where `x^2 < DBL_EPSILON`.
// R: arithmetic.c:Rexp
fn rexp(x: f64) -> f64 {
    if x.abs() <= f64::EPSILON.sqrt() { 1.0 + x } else { x.exp() }
}

// R: arithmetic.c:Rexpm1
fn rexpm1(x: f64) -> f64 {
    f_x_x(x, robigo_numerics::expm1, f64::EPSILON)
}

// R: arithmetic.c:Rlog1p
fn rlog1p(x: f64) -> f64 {
    f_x_x(x, robigo_numerics::log1p, f64::EPSILON)
}

// R: arithmetic.c:Rsin
fn rsin(x: f64) -> f64 {
    f_x_x(x, f64::sin, (3.0 * f64::EPSILON).sqrt())
}

// R: arithmetic.c:Rtan
fn rtan(x: f64) -> f64 {
    f_x_x(x, f64::tan, (1.5 * f64::EPSILON).sqrt())
}

/// `Rcos(x)`: `1 - x^2/2` where `x^4 < 12 DBL_EPSILON`.
// R: arithmetic.c:Rcos
fn rcos(x: f64) -> f64 {
    if x.abs() < (12.0 * f64::EPSILON).sqrt().sqrt() { 1.0 - x * x * 0.5 } else { x.cos() }
}

// R: arithmetic.c:Rasin
fn rasin(x: f64) -> f64 {
    f_x_x(x, f64::asin, (3.0 * f64::EPSILON).sqrt())
}

// R: arithmetic.c:Ratan
fn ratan(x: f64) -> f64 {
    f_x_x(x, f64::atan, (1.5 * f64::EPSILON).sqrt())
}

/// `math1_ari(sa, f, arg, res, lcall)`'s element: `res` where `x == arg`
/// (so `-0` too), else `f(x)`. One function per row, so that `math1`
/// takes a plain function pointer as GNU R's does.
macro_rules! ari {
    ($($name:ident = $f:expr, $arg:literal, $res:literal;)*) => {$(
        // R: arithmetic.c:math1_ari
        fn $name(x: f64) -> f64 {
            if x == $arg { $res } else { $f(x) }
        }
    )*};
}

ari! {
    exp_ari = rexp, 0.0, 1.0;
    expm1_ari = rexpm1, 0.0, 0.0;
    log1p_ari = rlog1p, 0.0, 0.0;
    cos_ari = rcos, 0.0, 1.0;
    sin_ari = rsin, 0.0, 0.0;
    tan_ari = rtan, 0.0, 0.0;
    acos_ari = f64::acos, 1.0, 0.0;
    asin_ari = rasin, 0.0, 0.0;
    atan_ari = ratan, 0.0, 0.0;
    cosh_ari = f64::cosh, 0.0, 1.0;
    sinh_ari = f64::sinh, 0.0, 0.0;
    tanh_ari = f64::tanh, 0.0, 0.0;
    acosh_ari = robigo_numerics::acosh, 1.0, 0.0;
    asinh_ari = robigo_numerics::asinh, 0.0, 0.0;
    atanh_ari = robigo_numerics::atanh, 0.0, 0.0;
}

/// nmath's `sign(x)`: -1, 0 or 1; NaN stays.
// R: nmath/sign.c:sign
fn sign(x: f64) -> f64 {
    if x.is_nan() {
        x
    } else if x > 0.0 {
        1.0
    } else if x == 0.0 {
        0.0
    } else {
        -1.0
    }
}

/// One of `do_math1`'s rows, by its names.c code.
struct Math1<const CODE: i32>;

impl<const CODE: i32> Math1<CODE> {
    /// The row's name, its `PRIMNAME`.
    const NAME: &'static str = match CODE {
        1 => "floor",
        2 => "ceiling",
        3 => "sqrt",
        4 => "sign",
        10 => "exp",
        11 => "expm1",
        12 => "log1p",
        20 => "cos",
        21 => "sin",
        22 => "tan",
        23 => "acos",
        24 => "asin",
        25 => "atan",
        30 => "cosh",
        31 => "sinh",
        32 => "tanh",
        33 => "acosh",
        34 => "asinh",
        35 => "atanh",
        40 => "lgamma",
        41 => "gamma",
        42 => "digamma",
        43 => "trigamma",
        47 => "cospi",
        48 => "sinpi",
        49 => "tanpi",
        _ => panic!("robigo: not a do_math1 code"),
    };
    const GENERIC: Generic = Generic::group("Math", Self::NAME);
    const ROW: &'static Builtin = crate::registry::find(Self::NAME, None);
    /// The element function (`MATH1(f)` or a `math1_ari` row's), and
    /// whether it is nmath's, which may warn (`ML_WARNING`).
    const KERNEL: (fn(f64) -> f64, bool) = match CODE {
        1 => (f64::floor, false),
        2 => (f64::ceil, false),
        3 => (rsqrt, false),
        4 => (sign, false),
        10 => (exp_ari, false),
        11 => (expm1_ari, false),
        12 => (log1p_ari, false),
        20 => (cos_ari, false),
        21 => (sin_ari, false),
        22 => (tan_ari, false),
        23 => (acos_ari, false),
        24 => (asin_ari, false),
        25 => (atan_ari, false),
        30 => (cosh_ari, false),
        31 => (sinh_ari, false),
        32 => (tanh_ari, false),
        33 => (acosh_ari, false),
        34 => (asinh_ari, false),
        35 => (atanh_ari, false),
        40 => (robigo_numerics::lgammafn, true),
        41 => (robigo_numerics::gammafn, true),
        42 => (robigo_numerics::digamma, true),
        43 => (robigo_numerics::trigamma, true),
        47 => (robigo_numerics::cospi, true),
        48 => (robigo_numerics::sinpi, true),
        // "our own in any case"
        49 => (robigo_numerics::r_tanpi, true),
        _ => panic!("robigo: not a do_math1 code"),
    };
}

/// `floor(x)`, `sqrt(x)`, `gamma(x)`, ...: `do_math1`'s rows, by names.c
/// code.
// R: arithmetic.c:do_math1
pub fn do_math1<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    Math1::<CODE>::ROW.check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let Dispatched::Default(out) = math_dispatch(cx, &Math1::<CODE>::GENERIC, call, args, env, out)? else {
        return Ok(());
    };
    if args.get(cx, 0).sexptype() == CPLXSXP {
        let what = format!("{}() of a complex vector (complex_math1)", Math1::<CODE>::NAME);
        return Err(not_yet(cx, Some(call), &what));
    }
    let (f, nmath) = Math1::<CODE>::KERNEL;
    math1(cx, call, args.root(0), f, nmath, out)
}

const TRUNC: Generic = Generic::group("Math", "trunc");

/// `trunc(x, ...)`: methods may take more arguments; the default reads
/// the first.
// R: arithmetic.c:do_trunc
pub fn do_trunc(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Dispatched::Default(out) = math_dispatch(cx, &TRUNC, call, args, env, out)? else { return Ok(()) };
    // checkArity(op, args): its arity is -1 in names.c
    crate::util::check1arg(cx, args, call, "x")?;
    if args.is_empty() {
        // CAR(R_NilValue) is R_NilValue: not numeric
        return Err(cx.error_call(Some(call), NONNUM));
    }
    if args.get(cx, 0).sexptype() == CPLXSXP {
        return Err(cx.error_call(Some(call), "unimplemented complex function"));
    }
    math1(cx, call, args.root(0), f64::trunc, false, out)
}

const ABS: Generic = Generic::group("Math", "abs");

/// `abs(x)`: integer for integer or logical input, double for double.
// R: arithmetic.c:do_abs
pub fn do_abs(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("abs").check_arity(cx, call, args)?;
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
    // PLANT(A4, crash): a panic on one input class, which must cost one case
    assert!(
        !(cfg!(feature = "diff-plant") && x.bind(cx).sexptype() == INTSXP && x.bind(cx).len() == Some(3)),
        "robigo: A4 planted crash"
    );
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
