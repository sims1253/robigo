//! Port of GNU R's numerical library (`src/nmath`): special functions,
//! rounding, distribution functions and random variate generators.
//!
//! The C sources are the spec; each function names the file it ports.
//! Functions are plain `f64 -> f64` maps like their C counterparts.
//! Warnings that nmath raises through `MATHLIB_WARNING` (R's `warning()`)
//! are collected in a thread-local buffer; builtins drain it with
//! [`take_warnings`] and emit them against the right call.

#![allow(
    non_snake_case,
    clippy::excessive_precision,
    clippy::many_single_char_names,
    clippy::too_many_arguments,
    clippy::neg_cmp_op_on_partial_ord
)]

use std::cell::RefCell;

/// R's `NA_real_`: a NaN whose low word is 1954.
pub const NA_REAL: f64 = f64::from_bits(0x7FF0_0000_0000_07A2);

pub mod arith;
pub mod bessel;
pub mod beta;
pub mod binom;
pub mod dist;
pub mod fdist;
pub mod fround;
pub mod gamma;
pub mod hyper;
pub mod ldmath;
pub mod ldouble;
pub mod nbeta;
pub mod nbinom;
pub mod nchisq;
pub mod nf;
pub mod norm;
pub mod nt;
pub mod pgamma;
pub mod policy;
pub mod polygamma;
pub mod rand;
pub mod rng;
pub mod signrank;
pub mod tdist;
pub mod toms708;
pub mod tukey;
pub mod wilcox;

pub use self::gamma::{beta as beta_fn, choose, gammafn, lbeta, lchoose, lgammafn, lgammafn_sign};
pub use self::polygamma::{digamma, psigamma, trigamma};

// ----------------------------------------------------------------------
// Constants (Rmath.h)

pub const M_E: f64 = 2.718281828459045235360287471353;
pub const M_LN2: f64 = 0.693147180559945309417232121458;
pub const M_LN10: f64 = 2.302585092994045684017991454684;
pub const M_PI: f64 = 3.141592653589793238462643383280;
pub const M_2PI: f64 = 6.283185307179586476925286766559;
pub const M_PI_2: f64 = 1.570796326794896619231321691640;
pub const M_1_PI: f64 = 0.318309886183790671537767526745;
pub const M_SQRT2: f64 = 1.414213562373095048801688724210;
pub const M_SQRT_32: f64 = 5.656854249492380195206754896838;
pub const M_LOG10_2: f64 = 0.301029995663981195213738894724;
pub const M_SQRT_PI: f64 = 1.772453850905516027298167483341;
pub const M_1_SQRT_2PI: f64 = 0.398942280401432677939946059934;
pub const M_SQRT_2DPI: f64 = 0.797884560802865355879892119869;
pub const M_LN_2PI: f64 = 1.837877066409345483560659472811;
pub const M_LN_SQRT_PI: f64 = 0.572364942924700087071713675677;
pub const M_LN_SQRT_2PI: f64 = 0.918938533204672741780329736406;
pub const M_LN_SQRT_PID2: f64 = 0.225791352644727432363097614947;

pub const DBL_EPSILON: f64 = f64::EPSILON;
pub const DBL_MIN: f64 = f64::MIN_POSITIVE;
pub const DBL_MAX: f64 = f64::MAX;
pub const DBL_MANT_DIG: i32 = 53;
pub const DBL_MIN_EXP: i32 = -1021;
pub const DBL_MAX_EXP: i32 = 1024;
pub const DBL_DIG: i32 = 15;
pub const DBL_MAX_10_EXP: i32 = 308;
pub const INT_MAX: i32 = i32::MAX;

pub const ML_NAN: f64 = f64::NAN;
pub const ML_POSINF: f64 = f64::INFINITY;
pub const ML_NEGINF: f64 = f64::NEG_INFINITY;

// ----------------------------------------------------------------------
// C library functions GNU R uses that Rust's std does not expose. nmath
// calls the C library's `lgamma` in places, and GNU R 4.5.0 uses the C23
// `sinpi`/`cospi`/`tanpi`/`atanpi` when the C library has them (glibc >=
// 2.41, as for the oracle, Debian 13's r-base-core) and its own versions
// in nmath/cospi.c otherwise. robigo does the same at run time, so one
// binary runs on any glibc and matches GNU R built for that system.

mod c23 {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;

    pub type F = unsafe extern "C" fn(f64) -> f64;

    #[cfg(target_os = "linux")]
    fn lookup(name: &CStr) -> Option<F> {
        unsafe extern "C" {
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }
        // RTLD_DEFAULT is the null handle on glibc.
        // SAFETY: dlsym with a NUL-terminated name; a non-null result is the
        // address of the C library's `double f(double)`.
        let p = unsafe { dlsym(std::ptr::null_mut(), name.as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, F>(p) })
    }

    #[cfg(not(target_os = "linux"))]
    fn lookup(_: &CStr) -> Option<F> {
        None
    }

    pub fn get(cell: &OnceLock<Option<F>>, name: &CStr) -> Option<F> {
        *cell.get_or_init(|| lookup(name))
    }

    pub static SINPI: OnceLock<Option<F>> = OnceLock::new();
    pub static COSPI: OnceLock<Option<F>> = OnceLock::new();
    pub static TANPI: OnceLock<Option<F>> = OnceLock::new();
    pub static ATANPI: OnceLock<Option<F>> = OnceLock::new();
}

mod libm {
    unsafe extern "C" {
        pub fn lgamma_r(x: f64, sign: *mut i32) -> f64;
        pub fn logb(x: f64) -> f64;
        pub fn ldexp(x: f64, e: i32) -> f64;
        pub fn nearbyint(x: f64) -> f64;
        pub fn expm1(x: f64) -> f64;
        pub fn log1p(x: f64) -> f64;
    }
}

/// `sin(pi * x)`: the C library's `sinpi`, else GNU R's.
#[inline]
pub fn sinpi(x: f64) -> f64 {
    match c23::get(&c23::SINPI, c"sinpi") {
        // SAFETY: the C library's `double sinpi(double)`.
        Some(f) => unsafe { f(x) },
        None => r_sinpi(x),
    }
}

/// `cos(pi * x)`: the C library's `cospi`, else GNU R's.
#[inline]
pub fn cospi(x: f64) -> f64 {
    match c23::get(&c23::COSPI, c"cospi") {
        // SAFETY: the C library's `double cospi(double)`.
        Some(f) => unsafe { f(x) },
        None => r_cospi(x),
    }
}

/// `tan(pi * x)`: the C library's `tanpi`, else GNU R's `Rtanpi`.
#[inline]
pub fn tanpi(x: f64) -> f64 {
    match c23::get(&c23::TANPI, c"tanpi") {
        // SAFETY: the C library's `double tanpi(double)`.
        Some(f) => unsafe { f(x) },
        None => r_tanpi(x),
    }
}

/// `atan(x) / pi`: the C library's `atanpi`, else what GNU R's pcauchy.c
/// computes without it.
#[inline]
pub fn atanpi(x: f64) -> f64 {
    match c23::get(&c23::ATANPI, c"atanpi") {
        // SAFETY: the C library's `double atanpi(double)`.
        Some(f) => unsafe { f(x) },
        None => x.atan() / std::f64::consts::PI,
    }
}

/// GNU R's `cospi` for C libraries without one: exact for x = k/2.
// R: nmath/cospi.c:cospi
pub fn r_cospi(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if !x.is_finite() {
        return f64::NAN;
    }
    let x = (x.abs()) % 2.0;
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

/// GNU R's `sinpi` for C libraries without one: exact for x = k/2.
// R: nmath/cospi.c:sinpi
pub fn r_sinpi(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
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

/// R: nmath/cospi.c:Rtanpi -- R's own tan(pi * x), used by `tanpi()` in
/// arithmetic.c.
pub fn r_tanpi(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if !x.is_finite() {
        return ML_NAN;
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
        ML_NAN
    } else if x == 0.25 {
        1.0
    } else if x == -0.25 {
        -1.0
    } else {
        (M_PI * x).tan()
    }
}

/// The C library's `lgamma` (log |Gamma(x)|), distinct from nmath's
/// `lgammafn`.
#[inline]
pub fn c_lgamma(x: f64) -> f64 {
    let mut s = 0i32;
    unsafe { libm::lgamma_r(x, &mut s) }
}

#[inline]
pub fn logb(x: f64) -> f64 {
    unsafe { libm::logb(x) }
}

#[inline]
pub fn ldexp(x: f64, e: i32) -> f64 {
    unsafe { libm::ldexp(x, e) }
}

/// C `nearbyint` in the default rounding mode (ties to even).
#[inline]
pub fn nearbyint(x: f64) -> f64 {
    unsafe { libm::nearbyint(x) }
}

#[inline]
pub fn expm1(x: f64) -> f64 {
    unsafe { libm::expm1(x) }
}

#[inline]
pub fn log1p(x: f64) -> f64 {
    unsafe { libm::log1p(x) }
}

/// C `round`: half away from zero.
#[inline]
pub fn c_round(x: f64) -> f64 {
    x.round()
}

/// C `fmod`.
#[inline]
pub fn fmod(x: f64, y: f64) -> f64 {
    x % y
}

// ----------------------------------------------------------------------
// nmath.h helpers

#[inline]
pub fn r_forceint(x: f64) -> f64 {
    nearbyint(x)
}

/// `R_nonint(x)`: not (nearly) an integer.
#[inline]
pub fn r_nonint(x: f64) -> bool {
    (x - r_forceint(x)).abs() > 1e-9 * fmax2(1.0, x.abs())
}

/// R: nmath/fmax2.c
#[inline]
pub fn fmax2(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if x < y { y } else { x }
}

/// R: nmath/fmin2.c
#[inline]
pub fn fmin2(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if x < y { x } else { y }
}

#[inline]
pub fn imax2(x: i32, y: i32) -> i32 {
    if x < y { y } else { x }
}

#[inline]
pub fn imin2(x: i32, y: i32) -> i32 {
    if x < y { x } else { y }
}

/// R: nmath/fsign.c
pub fn fsign(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if y >= 0.0 { x.abs() } else { -x.abs() }
}

/// R: main/arithmetic.c:R_pow
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
            return ML_POSINF;
        } else {
            return y;
        }
    }
    if x.is_finite() && y.is_finite() {
        return x.powf(y);
    }
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if !x.is_finite() {
        if x > 0.0 {
            return if y < 0.0 { 0.0 } else { ML_POSINF };
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
            return if x >= 1.0 { ML_POSINF } else { 0.0 };
        } else {
            return if x < 1.0 { ML_POSINF } else { 0.0 };
        }
    }
    ML_NAN
}

use crate::arith::myfmod;

/// R: main/arithmetic.c:R_pow_di
pub fn r_pow_di(mut x: f64, mut n: i32) -> f64 {
    let mut xn = 1.0;
    if x.is_nan() {
        return x;
    }
    if n == i32::MIN {
        return crate::NA_REAL;
    }
    if n != 0 {
        if !x.is_finite() {
            return r_pow(x, n as f64);
        }
        let is_neg = n < 0;
        if is_neg {
            n = -n;
        }
        loop {
            if n & 1 != 0 {
                xn *= x;
            }
            n >>= 1;
            if n != 0 {
                x *= x;
            } else {
                break;
            }
        }
        if is_neg {
            xn = 1.0 / xn;
        }
    }
    xn
}

// ----------------------------------------------------------------------
// Warnings (nmath.h ML_WARNING / MATHLIB_WARNING)

thread_local! {
    static WARNINGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `MATHLIB_WARNING(fmt, ...)`: R's `warning()` with the formatted text.
pub fn mathlib_warning(msg: String) {
    WARNINGS.with(|w| w.borrow_mut().push(msg));
}

pub const ME_DOMAIN: i32 = 1;
pub const ME_RANGE: i32 = 2;
pub const ME_NOCONV: i32 = 4;
pub const ME_PRECISION: i32 = 8;
pub const ME_UNDERFLOW: i32 = 16;

/// `ML_WARNING(x, s)`: domain errors are silent (callers collect NaNs).
pub fn ml_warning(x: i32, s: &str) {
    if x > ME_DOMAIN {
        let msg = match x {
            ME_RANGE => format!("value out of range in '{}'\n", s),
            ME_NOCONV => format!("convergence failed in '{}'\n", s),
            ME_PRECISION => format!("full precision may not have been achieved in '{}'\n", s),
            ME_UNDERFLOW => format!("underflow occurred in '{}'\n", s),
            _ => String::new(),
        };
        mathlib_warning(msg);
    }
}

/// Take the warnings raised since the last call.
pub fn take_warnings() -> Vec<String> {
    WARNINGS.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

// ----------------------------------------------------------------------
// Allocation failures (nmath's calloc/malloc wrappers)

thread_local! {
    static ALLOC_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// R: main/memory.c:R_chk_calloc / R_chk_malloc.  nmath has no error
/// path, so a failing allocation records R's message here and returns
/// NaN; the `.Call` wrapper drains it with [`take_alloc_error`] and
/// raises the error, which aborts the call as in R.
pub fn alloc_error(nelem: u64, elsize: u64) -> f64 {
    ALLOC_ERROR.with(|e| {
        *e.borrow_mut() = Some(format!("'R_Calloc' could not allocate memory ({} of {} bytes)", nelem, elsize))
    });
    ML_NAN
}

/// Take the allocation error raised since the last call, if any.
pub fn take_alloc_error() -> Option<String> {
    ALLOC_ERROR.with(|e| e.borrow_mut().take())
}

/// C's `(int)` conversion of a double (`cvttsd2si`): out of range and NaN
/// give `INT_MIN`.  Rust's `as i32` saturates instead, which nmath's
/// integer arithmetic (e.g. `w_init_maybe`'s `(int) Inf`) does not.
#[inline]
pub fn c_int_cast(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 { i32::MIN } else { x as i32 }
}

/// C's `%f` (six decimals).
pub fn fmt_f(x: f64) -> String {
    format!("{:.6}", x)
}

/// C's `%g` (six significant digits, trailing zeros dropped).
pub fn fmt_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let p = 6usize;
    // exponent after rounding to p significant digits
    let e = format!("{:.*e}", p - 1, x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let strip = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
    };
    if exp < -4 || exp >= p as i32 {
        format!("{}e{}{:02}", strip(mant), if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let dec = (p as i32 - 1 - exp).max(0) as usize;
        strip(&format!("{:.*}", dec, x))
    }
}

// ----------------------------------------------------------------------
// dpq.h: density/probability/quantile helpers. `lower_tail` and `log_p`
// are passed explicitly where the C macros pick them up from scope.

#[inline]
pub fn r_d__0(log_p: bool) -> f64 {
    if log_p { ML_NEGINF } else { 0.0 }
}

#[inline]
pub fn r_d__1(log_p: bool) -> f64 {
    if log_p { 0.0 } else { 1.0 }
}

#[inline]
pub fn r_dt_0(lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d__0(log_p) } else { r_d__1(log_p) }
}

#[inline]
pub fn r_dt_1(lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d__1(log_p) } else { r_d__0(log_p) }
}

#[inline]
pub fn r_d_half(log_p: bool) -> f64 {
    if log_p { -M_LN2 } else { 0.5 }
}

#[inline]
pub fn r_d_lval(p: f64, lower_tail: bool) -> f64 {
    if lower_tail { p } else { 0.5 - p + 0.5 }
}

#[inline]
pub fn r_d_cval(p: f64, lower_tail: bool) -> f64 {
    if lower_tail { 0.5 - p + 0.5 } else { p }
}

#[inline]
pub fn r_d_val(x: f64, log_p: bool) -> f64 {
    if log_p { x.ln() } else { x }
}

#[inline]
pub fn r_d_qiv(p: f64, log_p: bool) -> f64 {
    if log_p { p.exp() } else { p }
}

#[inline]
pub fn r_d_exp(x: f64, log_p: bool) -> f64 {
    if log_p { x } else { x.exp() }
}

#[inline]
pub fn r_d_log(p: f64, log_p: bool) -> f64 {
    if log_p { p } else { p.ln() }
}

#[inline]
pub fn r_d_clog(p: f64, log_p: bool) -> f64 {
    if log_p { log1p(-p) } else { 0.5 - p + 0.5 }
}

/// `R_Log1_Exp(x)`: log(1 - exp(x)).
#[inline]
pub fn r_log1_exp(x: f64) -> f64 {
    if x > -M_LN2 { (-expm1(x)).ln() } else { log1p(-x.exp()) }
}

#[inline]
pub fn r_d_lexp(x: f64, log_p: bool) -> f64 {
    if log_p { r_log1_exp(x) } else { log1p(-x) }
}

#[inline]
pub fn r_dt_val(x: f64, lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d_val(x, log_p) } else { r_d_clog(x, log_p) }
}

#[inline]
pub fn r_dt_cval(x: f64, lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d_clog(x, log_p) } else { r_d_val(x, log_p) }
}

#[inline]
pub fn r_dt_qiv(p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if log_p { if lower_tail { p.exp() } else { -expm1(p) } } else { r_d_lval(p, lower_tail) }
}

#[inline]
pub fn r_dt_civ(p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if log_p { if lower_tail { -expm1(p) } else { p.exp() } } else { r_d_cval(p, lower_tail) }
}

#[inline]
pub fn r_dt_exp(x: f64, lower_tail: bool, log_p: bool) -> f64 {
    r_d_exp(r_d_lval(x, lower_tail), log_p)
}

#[inline]
pub fn r_dt_cexp(x: f64, lower_tail: bool, log_p: bool) -> f64 {
    r_d_exp(r_d_cval(x, lower_tail), log_p)
}

#[inline]
pub fn r_dt_log(p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d_log(p, log_p) } else { r_d_lexp(p, log_p) }
}

#[inline]
pub fn r_dt_clog(p: f64, lower_tail: bool, log_p: bool) -> f64 {
    if lower_tail { r_d_lexp(p, log_p) } else { r_d_log(p, log_p) }
}

#[inline]
pub fn r_dt_log_(p: f64, lower_tail: bool) -> f64 {
    if lower_tail { p } else { r_log1_exp(p) }
}

/// `R_Q_P01_check(p)`: true when p is out of range (caller returns NaN).
#[inline]
pub fn r_q_p01_check(p: f64, log_p: bool) -> bool {
    (log_p && p > 0.0) || (!log_p && !(0.0..=1.0).contains(&p))
}

/// `R_Q_P01_boundaries(p, LEFT, RIGHT)`: `Some(value)` when the C macro
/// returns early.
#[inline]
pub fn r_q_p01_boundaries(p: f64, left: f64, right: f64, lower_tail: bool, log_p: bool) -> Option<f64> {
    if log_p {
        if p > 0.0 {
            return Some(ML_NAN);
        }
        if p == 0.0 {
            return Some(if lower_tail { right } else { left });
        }
        if p == ML_NEGINF {
            return Some(if lower_tail { left } else { right });
        }
    } else {
        if !(0.0..=1.0).contains(&p) {
            return Some(ML_NAN);
        }
        if p == 0.0 {
            return Some(if lower_tail { left } else { right });
        }
        if p == 1.0 {
            return Some(if lower_tail { right } else { left });
        }
    }
    None
}

#[inline]
pub fn r_d_fexp(f: f64, x: f64, log_p: bool) -> f64 {
    if log_p { -0.5 * f.ln() + x } else { x.exp() / f.sqrt() }
}

#[inline]
pub fn r_d_rtxp(rf: f64, x: f64, log_p: bool) -> f64 {
    if log_p { -rf.ln() + x } else { x.exp() / rf }
}

#[inline]
pub fn r_d_neg_i_nonint(x: f64) -> bool {
    x < 0.0 || r_nonint(x)
}

/// `R_D_nonint_check(x)`: warns and returns `Some(R_D__0)` for non-integer x.
#[inline]
pub fn r_d_nonint_check(x: f64, log_p: bool) -> Option<f64> {
    if r_nonint(x) {
        mathlib_warning(format!("non-integer x = {}", fmt_f(x)));
        return Some(r_d__0(log_p));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow_di() {
        assert_eq!(r_pow_di(10.0, 3), 1000.0);
        assert_eq!(r_pow_di(10.0, -2), 0.01);
        assert_eq!(r_pow_di(2.0, 0), 1.0);
    }

    #[test]
    fn libm_pi_functions() {
        assert_eq!(sinpi(1.0), 0.0);
        assert_eq!(cospi(0.5), 0.0);
        assert_eq!(r_tanpi(0.25), 1.0);
        assert_eq!(logb(8.0), 3.0);
        assert_eq!(nearbyint(2.5), 2.0);
        assert_eq!(nearbyint(3.5), 4.0);
    }
}
