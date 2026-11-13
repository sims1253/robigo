//! complex.c's arithmetic, bit for bit with the C library GNU R runs on:
//! libgcc's `__muldc3`/`__divdc3` for C99's `*` and `/` on `double
//! complex`, glibc's `clog`/`cexp` for `cpow`, and R's own `mycpow`/
//! `R_cpow_n` for `^`. Every operation goes through [`super::nan`], so NaN
//! payloads (NA against NaN) are those of the compiled C; the operand
//! orders below are gcc 14's, measured by the old runtime's oracle tests
//! (`crates/robigo-runtime/src/builtins/complex.rs`, which this ports to
//! safe code) and re-checked on 4.6.2 by this crate's tests.

use super::arith::r_pow;
use super::nan::{add, div, mul, sub};
use robigo_rtcore::Rcomplex;

// libgcc2.c's constants for the double complex routines.
const RBIG: f64 = f64::MAX / 2.0;
const RMIN: f64 = f64::MIN_POSITIVE;
const RMIN2: f64 = f64::EPSILON;
const RMINSCAL: f64 = 1.0 / f64::EPSILON;
const RMAX2: f64 = RBIG * RMIN2;

#[inline(always)]
fn c(r: f64, i: f64) -> Rcomplex {
    Rcomplex { r, i }
}

/// complex_binary's `x1 + x2`: gcc commuted the compiled adds, so the
/// second operand's NaN payload wins (`(0/0+1i) + (1+NA)` is NA).
// R: complex.c:complex_binary
#[inline(always)]
pub fn cadd(a: Rcomplex, b: Rcomplex) -> Rcomplex {
    c(add(b.r, a.r), add(b.i, a.i))
}

/// complex_binary's `x1 - x2`.
// R: complex.c:complex_binary
#[inline(always)]
pub fn csub(a: Rcomplex, b: Rcomplex) -> Rcomplex {
    c(sub(a.r, b.r), sub(a.i, b.i))
}

/// complex_binary's `x1 * x2`, which gcc compiles to `__muldc3(x2, x1)`.
// R: complex.c:complex_binary
#[inline(always)]
pub fn ctimes(a: Rcomplex, b: Rcomplex) -> Rcomplex {
    cmul(b, a)
}

/// libgcc's `__muldc3(a, b)`: C99's `a * b`.
// R: libgcc2.c:__muldc3
pub fn cmul(x: Rcomplex, y: Rcomplex) -> Rcomplex {
    let (a, b, cc, d) = (x.r, x.i, y.r, y.i);
    let ac = mul(a, cc);
    let bd = mul(b, d);
    let ad = mul(a, d);
    // gcc 14's compiled __muldc3 computes `c * b` (the destination of its
    // mulsd holds c): with two NaN operands c's payload wins.
    let bc = mul(cc, b);
    let x = sub(ac, bd);
    let y = add(ad, bc);
    let inf = |v: f64| v.is_infinite();
    if x.is_nan() && y.is_nan() && (inf(a) | inf(b) | inf(cc) | inf(d) | inf(ac) | inf(bd) | inf(ad) | inf(bc)) {
        return cmul_recover([a, b, cc, d], [ac, bd, ad, bc], x, y);
    }
    c(x, y)
}

/// `__muldc3`'s recovery of infinities that computed as NaN + iNaN.
#[cold]
#[inline(never)]
fn cmul_recover(
    [mut a, mut b, mut cc, mut d]: [f64; 4],
    [ac, bd, ad, bc]: [f64; 4],
    mut x: f64,
    mut y: f64,
) -> Rcomplex {
    let mut recalc = false;
    let boxed = |v: f64| f64::copysign(if v.is_infinite() { 1.0 } else { 0.0 }, v);
    let zero_nan = |v: f64| if v.is_nan() { f64::copysign(0.0, v) } else { v };
    if a.is_infinite() || b.is_infinite() {
        a = boxed(a);
        b = boxed(b);
        cc = zero_nan(cc);
        d = zero_nan(d);
        recalc = true;
    }
    if cc.is_infinite() || d.is_infinite() {
        cc = boxed(cc);
        d = boxed(d);
        a = zero_nan(a);
        b = zero_nan(b);
        recalc = true;
    }
    if !recalc && (ac.is_infinite() || bd.is_infinite() || ad.is_infinite() || bc.is_infinite()) {
        a = zero_nan(a);
        b = zero_nan(b);
        cc = zero_nan(cc);
        d = zero_nan(d);
        recalc = true;
    }
    if recalc {
        x = mul(f64::INFINITY, sub(mul(a, cc), mul(b, d)));
        y = mul(f64::INFINITY, add(mul(a, d), mul(b, cc)));
    }
    c(x, y)
}

/// libgcc's `__divdc3(a, b)`: C99's `a / b` (gcc 12+'s scaled Smith
/// algorithm).
// R: libgcc2.c:__divdc3
pub fn cdiv(x: Rcomplex, y: Rcomplex) -> Rcomplex {
    let (mut a, mut b, mut cc, mut d) = (x.r, x.i, y.r, y.i);
    let (mut re, mut im);
    if cc.abs() < d.abs() {
        // Prevent underflow when the denominator is near the largest value.
        if d.abs() >= RBIG {
            a = div(a, 2.0);
            b = div(b, 2.0);
            cc = div(cc, 2.0);
            d = div(d, 2.0);
        }
        // Avoid overflow and underflow when c and d are small.
        if d.abs() < RMIN2
            || (a.abs() < RMIN && b.abs() < RMAX2 && d.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && d.abs() < RMAX2)
        {
            a = mul(a, RMINSCAL);
            b = mul(b, RMINSCAL);
            cc = mul(cc, RMINSCAL);
            d = mul(d, RMINSCAL);
        }
        let ratio = div(cc, d);
        let denom = add(mul(cc, ratio), d);
        if ratio.abs() > RMIN {
            re = div(add(mul(a, ratio), b), denom);
            im = div(sub(mul(b, ratio), a), denom);
        } else {
            // gcc 14 emits `(a/d) * c`, not the source's `c * (a/d)`.
            re = div(add(mul(div(a, d), cc), b), denom);
            im = div(sub(mul(div(b, d), cc), a), denom);
        }
    } else {
        if cc.abs() >= RBIG {
            a = div(a, 2.0);
            b = div(b, 2.0);
            cc = div(cc, 2.0);
            d = div(d, 2.0);
        }
        if cc.abs() < RMIN2
            || (a.abs() < RMIN && b.abs() < RMAX2 && cc.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && cc.abs() < RMAX2)
        {
            a = mul(a, RMINSCAL);
            b = mul(b, RMINSCAL);
            cc = mul(cc, RMINSCAL);
            d = mul(d, RMINSCAL);
        }
        let ratio = div(d, cc);
        let denom = add(mul(d, ratio), cc);
        if ratio.abs() > RMIN {
            re = div(add(mul(b, ratio), a), denom);
            im = div(sub(b, mul(a, ratio)), denom);
        } else {
            // gcc 14 emits `(b/c) * d` with the product as destination.
            re = div(add(mul(div(b, cc), d), a), denom);
            im = div(sub(b, mul(div(a, cc), d)), denom);
        }
    }
    // Recover infinities and zeros that computed as NaN + iNaN: nonzero
    // over zero, infinite over finite, finite over infinite.
    if re.is_nan() && im.is_nan() {
        if cc == 0.0 && d == 0.0 && (!a.is_nan() || !b.is_nan()) {
            re = mul(f64::copysign(f64::INFINITY, cc), a);
            im = mul(f64::copysign(f64::INFINITY, cc), b);
        } else if (a.is_infinite() || b.is_infinite()) && cc.is_finite() && d.is_finite() {
            let a2 = f64::copysign(if a.is_infinite() { 1.0 } else { 0.0 }, a);
            let b2 = f64::copysign(if b.is_infinite() { 1.0 } else { 0.0 }, b);
            re = mul(f64::INFINITY, add(mul(a2, cc), mul(b2, d)));
            im = mul(f64::INFINITY, sub(mul(b2, cc), mul(a2, d)));
        } else if (cc.is_infinite() || d.is_infinite()) && a.is_finite() && b.is_finite() {
            let c2 = f64::copysign(if cc.is_infinite() { 1.0 } else { 0.0 }, cc);
            let d2 = f64::copysign(if d.is_infinite() { 1.0 } else { 0.0 }, d);
            re = mul(0.0, add(mul(a, c2), mul(b, d2)));
            im = mul(0.0, sub(mul(b, c2), mul(a, d2)));
        }
    }
    c(re, im)
}

/// `R_cpow_n(X, k)`: `X^k` for an integer `k` by binary powering.
// R: complex.c:R_cpow_n
fn r_cpow_n(mut x: Rcomplex, mut k: i32) -> Rcomplex {
    if k == 0 {
        return c(1.0, 0.0);
    } else if k == 1 {
        return x;
    } else if k < 0 {
        return cdiv(c(1.0, 0.0), r_cpow_n(x, -k));
    }
    let mut z = c(1.0, 0.0);
    while k > 0 {
        if k & 1 != 0 {
            z = cmul(z, x);
        }
        if k == 1 {
            break;
        }
        k >>= 1;
        x = cmul(x, x);
    }
    z
}

/// C's `(int)` of a double on x86-64 (`cvttsd2si`): out of range and NaN
/// give `INT_MIN`.
fn c_int(x: f64) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&x) { x as i32 } else { i32::MIN }
}

/// `mycpow(X, Y)`: complex.c's `^`.
// R: complex.c:mycpow
pub fn cpow(x: Rcomplex, y: Rcomplex) -> Rcomplex {
    let (yr, yi) = (y.r, y.i);
    if x.r == 0.0 && x.i == 0.0 {
        if yi == 0.0 { c(r_pow(0.0, yr), 0.0) } else { c(f64::NAN, f64::NAN) }
    } else {
        let k = c_int(yr);
        if yi == 0.0 && yr == k as f64 && k.unsigned_abs() <= 65536 {
            r_cpow_n(x, k)
        } else {
            // glibc: cpow(x, y) = cexp(y * clog(x))
            cexp(cmul(y, clog(x)))
        }
    }
}

/// `x^2 + y^2 - 1` without large cancellation: glibc's `__x2y2m1`, built
/// for x86-64 without `__FP_FAST_FMA` (Dekker multiplication).
// R: glibc dbl-64/x2y2m1.c:__x2y2m1
fn x2y2m1(x: f64, y: f64) -> f64 {
    fn mul_split(x: f64, y: f64) -> (f64, f64) {
        let hi = x * y;
        const C: f64 = ((1u64 << (f64::MANTISSA_DIGITS as u64).div_ceil(2)) + 1) as f64;
        let mut x1 = x * C;
        let mut y1 = y * C;
        x1 = (x - x1) + x1;
        y1 = (y - y1) + y1;
        let x2 = x - x1;
        let y2 = y - y1;
        let lo = (((x1 * y1 - hi) + x1 * y2) + x2 * y1) + x2 * y2;
        (hi, lo)
    }
    let (hi1, lo1) = mul_split(x, x);
    let (hi2, lo2) = mul_split(y, y);
    let mut vals = [lo1, hi1, lo2, hi2, -1.0];
    let cmp = |p: &f64, q: &f64| p.abs().partial_cmp(&q.abs()).unwrap_or(std::cmp::Ordering::Equal);
    vals.sort_by(cmp);
    for i in 0..=3usize {
        let hi = vals[i + 1] + vals[i];
        let lo = (vals[i + 1] - hi) + vals[i];
        vals[i] = lo;
        vals[i + 1] = hi;
        vals[i..].sort_by(cmp);
    }
    vals[4] + vals[3] + vals[2] + vals[1] + vals[0]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FpClass {
    Nan,
    Infinite,
    Zero,
    Subnormal,
    Normal,
}

/// glibc's `fpclassify`.
fn classify(x: f64) -> FpClass {
    if x.is_nan() {
        FpClass::Nan
    } else if x.is_infinite() {
        FpClass::Infinite
    } else if x == 0.0 {
        FpClass::Zero
    } else if x.abs() < f64::MIN_POSITIVE {
        FpClass::Subnormal
    } else {
        FpClass::Normal
    }
}

/// glibc's `clog`.
// R: glibc math/s_clog_template.c:__clog
pub fn clog(x: Rcomplex) -> Rcomplex {
    let rcls = classify(x.r);
    let icls = classify(x.i);
    if rcls == FpClass::Zero && icls == FpClass::Zero {
        let im = f64::copysign(if x.r.is_sign_negative() { std::f64::consts::PI } else { 0.0 }, x.i);
        return c(-1.0 / x.r.abs(), im);
    }
    if rcls == FpClass::Nan || icls == FpClass::Nan {
        let re = if rcls == FpClass::Infinite || icls == FpClass::Infinite { f64::INFINITY } else { f64::NAN };
        return c(re, f64::NAN);
    }
    let mut absx = x.r.abs();
    let mut absy = x.i.abs();
    let mut scale = 0i32;
    if absx < absy {
        std::mem::swap(&mut absx, &mut absy);
    }
    if absx > f64::MAX / 2.0 {
        scale = -1;
        absx = robigo_numerics::ldexp(absx, scale);
        absy = if absy >= f64::MIN_POSITIVE * 2.0 { robigo_numerics::ldexp(absy, scale) } else { 0.0 };
    } else if absx < f64::MIN_POSITIVE && absy < f64::MIN_POSITIVE {
        scale = f64::MANTISSA_DIGITS as i32;
        absx = robigo_numerics::ldexp(absx, scale);
        absy = robigo_numerics::ldexp(absy, scale);
    }
    let re = if absx == 1.0 && scale == 0 {
        (absy * absy).ln_1p() / 2.0
    } else if absx > 1.0 && absx < 2.0 && absy < 1.0 && scale == 0 {
        let mut d2m1 = (absx - 1.0) * (absx + 1.0);
        if absy >= f64::EPSILON {
            d2m1 += absy * absy;
        }
        d2m1.ln_1p() / 2.0
    } else if (0.5..1.0).contains(&absx) && absy < f64::EPSILON / 2.0 && scale == 0 {
        ((absx - 1.0) * (absx + 1.0)).ln_1p() / 2.0
    } else if (0.5..1.0).contains(&absx) && scale == 0 && absx * absx + absy * absy >= 0.5 {
        x2y2m1(absx, absy).ln_1p() / 2.0
    } else {
        absx.hypot(absy).ln() - scale as f64 * std::f64::consts::LN_2
    };
    c(re, x.i.atan2(x.r))
}

/// glibc's `cexp`.
// R: glibc math/s_cexp_template.c:__cexp
pub fn cexp(x: Rcomplex) -> Rcomplex {
    let rcls = classify(x.r);
    let icls = classify(x.i);
    let finite = |k: FpClass| k != FpClass::Nan && k != FpClass::Infinite;
    let sincos = |im: f64| if im.abs() > f64::MIN_POSITIVE { (im.sin(), im.cos()) } else { (im, 1.0) };
    if finite(rcls) {
        if !finite(icls) {
            return c(f64::NAN, f64::NAN);
        }
        let t = ((f64::MAX_EXP - 1) as f64 * std::f64::consts::LN_2) as i32;
        let mut re = x.r;
        let (mut sinix, mut cosix) = sincos(x.i);
        if re > t as f64 {
            let exp_t = (t as f64).exp();
            re -= t as f64;
            sinix *= exp_t;
            cosix *= exp_t;
            if re > t as f64 {
                re -= t as f64;
                sinix *= exp_t;
                cosix *= exp_t;
            }
        }
        if re > t as f64 {
            // overflow: the original real part exceeded 3t
            c(f64::MAX * cosix, f64::MAX * sinix)
        } else {
            let e = re.exp();
            c(e * cosix, e * sinix)
        }
    } else if rcls == FpClass::Infinite {
        if finite(icls) {
            let value = if x.r.is_sign_negative() { 0.0 } else { f64::INFINITY };
            if icls == FpClass::Zero {
                c(value, x.i)
            } else {
                let (sinix, cosix) = sincos(x.i);
                c(f64::copysign(value, cosix), f64::copysign(value, sinix))
            }
        } else if !x.r.is_sign_negative() {
            c(f64::INFINITY, x.i - x.i)
        } else {
            c(0.0, f64::copysign(0.0, x.i))
        }
    } else {
        // the real part is NaN: NaN + iNaN unless the imaginary part is 0
        c(f64::NAN, if icls == FpClass::Zero { x.i } else { f64::NAN })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn products_and_quotients() {
        let r = cmul(c(1.0, 2.0), c(3.0, 4.0));
        assert_eq!((r.r, r.i), (-5.0, 10.0));
        let q = cdiv(c(-5.0, 10.0), c(3.0, 4.0));
        assert_eq!((q.r, q.i), (1.0, 2.0));
        let p = cpow(c(0.0, 1.0), c(2.0, 0.0));
        assert_eq!((p.r, p.i), (-1.0, 0.0));
        // nonzero over zero recovers an infinity
        let z = cdiv(c(1.0, 0.0), c(0.0, 0.0));
        assert!(z.r.is_infinite());
    }
}
