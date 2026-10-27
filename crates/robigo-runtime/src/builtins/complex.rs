//! Complex kernels, bit-identical with the C library GNU R 4.5.0 runs
//! on: gcc's libgcc `__muldc3`/`__divdc3` for the C99 `*` and `/`
//! operators R's `complex_binary` uses, glibc's `clog`/`cexp`/`csqrt`
//! for the math functions, and R's own `mycpow`/`R_cpow_n` for `^`.

use crate::value::Complex;

// gcc libgcc2.c constants for the double complex routines.
const RBIG: f64 = f64::MAX / 2.0;
const RMIN: f64 = f64::MIN_POSITIVE;
const RMIN2: f64 = f64::EPSILON;
const RMINSCAL: f64 = 1.0 / f64::EPSILON;
const RMAX2: f64 = RBIG * RMIN2;

// R: libgcc2.c:__muldc3 (the C99 `x * y` R's complex_binary uses).
pub fn cmul(a: Complex, b: Complex) -> Complex {
    let (a, b, c, d) = (a.re, a.im, b.re, b.im);
    let ac = a * c;
    let bd = b * d;
    let ad = a * d;
    let bc = b * c;
    let mut x = ac - bd;
    let mut y = ad + bc;
    if x.is_nan() && y.is_nan() {
        // Recover infinities that computed as NaN + iNaN.
        let (mut a, mut b, mut c, mut d) = (a, b, c, d);
        let mut recalc = false;
        if a.is_infinite() || b.is_infinite() {
            a = copysign(if a.is_infinite() { 1.0 } else { 0.0 }, a);
            b = copysign(if b.is_infinite() { 1.0 } else { 0.0 }, b);
            if c.is_nan() {
                c = copysign(0.0, c);
            }
            if d.is_nan() {
                d = copysign(0.0, d);
            }
            recalc = true;
        }
        if c.is_infinite() || d.is_infinite() {
            c = copysign(if c.is_infinite() { 1.0 } else { 0.0 }, c);
            d = copysign(if d.is_infinite() { 1.0 } else { 0.0 }, d);
            if a.is_nan() {
                a = copysign(0.0, a);
            }
            if b.is_nan() {
                b = copysign(0.0, b);
            }
            recalc = true;
        }
        if !recalc && (ac.is_infinite() || bd.is_infinite() || ad.is_infinite() || bc.is_infinite()) {
            if a.is_nan() {
                a = copysign(0.0, a);
            }
            if b.is_nan() {
                b = copysign(0.0, b);
            }
            if c.is_nan() {
                c = copysign(0.0, c);
            }
            if d.is_nan() {
                d = copysign(0.0, d);
            }
            recalc = true;
        }
        if recalc {
            x = f64::INFINITY * (a * c - b * d);
            y = f64::INFINITY * (a * d + b * c);
        }
    }
    Complex::new(x, y)
}

/// C's `copysign` (Rust's has the same behavior).
fn copysign(x: f64, y: f64) -> f64 {
    f64::copysign(x, y)
}

// R: libgcc2.c:__divdc3 (the C99 `x / y` R's complex_binary uses).
pub fn cdiv(a: Complex, b: Complex) -> Complex {
    let (mut a, mut b, mut c, mut d) = (a.re, a.im, b.re, b.im);
    let (mut x, mut y);
    if c.abs() < d.abs() {
        // Prevent underflow when denominator is near max representable.
        if d.abs() >= RBIG {
            a /= 2.0;
            b /= 2.0;
            c /= 2.0;
            d /= 2.0;
        }
        // Avoid overflow/underflow issues when c and d are small.
        if d.abs() < RMIN2 {
            a *= RMINSCAL;
            b *= RMINSCAL;
            c *= RMINSCAL;
            d *= RMINSCAL;
        } else if (a.abs() < RMIN && b.abs() < RMAX2 && d.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && d.abs() < RMAX2)
        {
            a *= RMINSCAL;
            b *= RMINSCAL;
            c *= RMINSCAL;
            d *= RMINSCAL;
        }
        let ratio = c / d;
        let denom = (c * ratio) + d;
        // Choose alternate order of computation if ratio is subnormal.
        if ratio.abs() > RMIN {
            x = ((a * ratio) + b) / denom;
            y = ((b * ratio) - a) / denom;
        } else {
            // gcc 14's __divdc3 emits `(a/d) * c`, not the source's
            // `c * (a/d)`: with NaN operands the destination operand's
            // payload wins, so the order is observable in the bits.
            x = (((a / d) * c) + b) / denom;
            y = (((b / d) * c) - a) / denom;
        }
    } else {
        if c.abs() >= RBIG {
            a /= 2.0;
            b /= 2.0;
            c /= 2.0;
            d /= 2.0;
        }
        if c.abs() < RMIN2 {
            a *= RMINSCAL;
            b *= RMINSCAL;
            c *= RMINSCAL;
            d *= RMINSCAL;
        } else if (a.abs() < RMIN && b.abs() < RMAX2 && c.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && c.abs() < RMAX2)
        {
            a *= RMINSCAL;
            b *= RMINSCAL;
            c *= RMINSCAL;
            d *= RMINSCAL;
        }
        let ratio = d / c;
        let denom = (d * ratio) + c;
        if ratio.abs() > RMIN {
            x = ((b * ratio) + a) / denom;
            y = (b - (a * ratio)) / denom;
        } else {
            // gcc 14 emits `(b/c) * d`, not the source's `d * (b/c)`,
            // and adds with the product as destination: with NaN
            // operands the destination operand's payload wins, so the
            // order is observable in the bits.
            x = (((b / c) * d) + a) / denom;
            y = (b - ((a / c) * d)) / denom;
        }
    }
    // Recover infinities and zeros that computed as NaN+iNaN; the only
    // cases are nonzero/zero, infinite/finite, and finite/infinite.
    if x.is_nan() && y.is_nan() {
        if c == 0.0 && d == 0.0 && (!a.is_nan() || !b.is_nan()) {
            x = f64::copysign(f64::INFINITY, c) * a;
            y = f64::copysign(f64::INFINITY, c) * b;
        } else if (a.is_infinite() || b.is_infinite()) && c.is_finite() && d.is_finite() {
            let a2 = copysign(if a.is_infinite() { 1.0 } else { 0.0 }, a);
            let b2 = copysign(if b.is_infinite() { 1.0 } else { 0.0 }, b);
            x = f64::INFINITY * (a2 * c + b2 * d);
            y = f64::INFINITY * (b2 * c - a2 * d);
        } else if (c.is_infinite() || d.is_infinite()) && a.is_finite() && b.is_finite() {
            let c2 = copysign(if c.is_infinite() { 1.0 } else { 0.0 }, c);
            let d2 = copysign(if d.is_infinite() { 1.0 } else { 0.0 }, d);
            x = 0.0 * (a * c2 + b * d2);
            y = 0.0 * (b * c2 - a * d2);
        }
    }
    Complex::new(x, y)
}

// R: complex.c:R_cpow_n — X^k for integer k by binary powering.
fn r_cpow_n(mut x: Complex, mut k: i32) -> Complex {
    if k == 0 {
        return Complex::new(1.0, 0.0);
    } else if k == 1 {
        return x;
    } else if k < 0 {
        return cdiv(Complex::new(1.0, 0.0), r_cpow_n(x, -k));
    }
    // k > 0
    let mut z = Complex::new(1.0, 0.0);
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

/// C's `(int)double` cast on x86 (`cvttsd2si`: truncates; out of range
/// or NaN gives INT_MIN, which C leaves undefined).
fn c_int(x: f64) -> i32 {
    if x >= -2147483648.0 && x < 2147483648.0 { x as i32 } else { i32::MIN }
}

// R: complex.c:mycpow — `x^y` for complex operands.
pub fn cpow(x: Complex, y: Complex) -> Complex {
    let (yr, yi) = (y.re, y.im);
    if x.re == 0.0 && x.im == 0.0 {
        // X == 0.0 (both parts zero)
        if yi == 0.0 {
            let p = super::arith::r_pow(0.0, yr);
            Complex::new(p, 0.0)
        } else {
            Complex::new(f64::NAN, f64::NAN)
        }
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

/// `x^2 + y^2 - 1` without large cancellation (glibc's `__x2y2m1`,
/// built for x86-64 without `__FP_FAST_FMA`: Dekker multiplication).
// R: dbl-64/x2y2m1.c:__x2y2m1
fn x2y2m1(x: f64, y: f64) -> f64 {
    fn mul_split(x: f64, y: f64) -> (f64, f64) {
        let hi = x * y;
        const C: f64 = ((1u64 << (f64::MANTISSA_DIGITS as u64 + 1) / 2) + 1) as f64;
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
    // vals sorted by |v| with qsort(compare) (stable enough: glibc qsort
    // on these distinct magnitudes); then four add_split passes, each
    // re-sorting the remainder.
    let mut vals = [lo1, hi1, lo2, hi2, -1.0];
    let cmp = |p: &f64, q: &f64| {
        let (pd, qd) = (p.abs(), q.abs());
        if pd < qd {
            std::cmp::Ordering::Less
        } else if pd == qd {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Greater
        }
    };
    vals.sort_by(cmp);
    for i in 0..=3usize {
        // add_split(&vals[i+1], &vals[i], vals[i+1], vals[i])
        let hi = vals[i + 1] + vals[i];
        let lo = (vals[i + 1] - hi) + vals[i];
        vals[i] = lo;
        vals[i + 1] = hi;
        vals[i..].sort_by(cmp);
    }
    vals[4] + vals[3] + vals[2] + vals[1] + vals[0]
}

// R: math/s_clog_template.c:__clog
pub fn clog(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut result = Complex::new(0.0, 0.0);
    if rcls == FpClass::Zero && icls == FpClass::Zero {
        result.im = if x.re.is_sign_negative() { std::f64::consts::PI } else { 0.0 };
        result.im = f64::copysign(result.im, x.im);
        // Yes, the following line raises an exception.
        result.re = -1.0 / x.re.abs();
    } else if rcls != FpClass::Nan && icls != FpClass::Nan {
        let mut absx = x.re.abs();
        let mut absy = x.im.abs();
        let mut scale = 0i32;
        if absx < absy {
            std::mem::swap(&mut absx, &mut absy);
        }
        if absx > f64::MAX / 2.0 {
            scale = -1;
            absx = crate::nmath::ldexp(absx, scale);
            absy = if absy >= f64::MIN_POSITIVE * 2.0 { crate::nmath::ldexp(absy, scale) } else { 0.0 };
        } else if absx < f64::MIN_POSITIVE && absy < f64::MIN_POSITIVE {
            scale = f64::MANTISSA_DIGITS as i32;
            absx = crate::nmath::ldexp(absx, scale);
            absy = crate::nmath::ldexp(absy, scale);
        }
        if absx == 1.0 && scale == 0 {
            result.re = (absy * absy).ln_1p() / 2.0;
        } else if absx > 1.0 && absx < 2.0 && absy < 1.0 && scale == 0 {
            let mut d2m1 = (absx - 1.0) * (absx + 1.0);
            if absy >= f64::EPSILON {
                d2m1 += absy * absy;
            }
            result.re = d2m1.ln_1p() / 2.0;
        } else if absx < 1.0 && absx >= 0.5 && absy < f64::EPSILON / 2.0 && scale == 0 {
            let d2m1 = (absx - 1.0) * (absx + 1.0);
            result.re = d2m1.ln_1p() / 2.0;
        } else if absx < 1.0 && absx >= 0.5 && scale == 0 && absx * absx + absy * absy >= 0.5 {
            let d2m1 = x2y2m1(absx, absy);
            result.re = d2m1.ln_1p() / 2.0;
        } else {
            let d = absx.hypot(absy);
            result.re = d.ln() - scale as f64 * std::f64::consts::LN_2;
        }
        result.im = x.im.atan2(x.re);
    } else {
        result.im = f64::NAN;
        if rcls == FpClass::Infinite || icls == FpClass::Infinite {
            result.re = f64::INFINITY;
        } else {
            result.re = f64::NAN;
        }
    }
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FpClass {
    Nan,
    Infinite,
    Zero,
    Subnormal,
    Normal,
}

/// glibc's `fpclassify` for double.
fn f64_classify(x: f64) -> FpClass {
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

// R: math/s_cexp_template.c:__cexp
pub fn cexp(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut retval = Complex::new(0.0, 0.0);
    if rcls != FpClass::Nan && rcls != FpClass::Infinite {
        // Real part is finite.
        if icls != FpClass::Nan && icls != FpClass::Infinite {
            // Imaginary part is finite.
            let t = ((f64::MAX_EXP - 1) as f64 * std::f64::consts::LN_2) as i32;
            let (mut re, im) = (x.re, x.im);
            let (sinix, cosix);
            if im.abs() > f64::MIN_POSITIVE {
                sinix = im.sin();
                cosix = im.cos();
            } else {
                sinix = im;
                cosix = 1.0;
            }
            let mut sinix = sinix;
            let mut cosix = cosix;
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
                // Overflow (original real part of x > 3t).
                retval.re = f64::MAX * cosix;
                retval.im = f64::MAX * sinix;
            } else {
                let exp_val = re.exp();
                retval.re = exp_val * cosix;
                retval.im = exp_val * sinix;
            }
        } else {
            // Imaginary part is +-inf or NaN, real part finite:
            // NaN + iNaN.
            retval.re = f64::NAN;
            retval.im = f64::NAN;
        }
    } else if rcls == FpClass::Infinite {
        // Real part is infinite.
        if icls != FpClass::Nan && icls != FpClass::Infinite {
            let value = if x.re.is_sign_negative() { 0.0 } else { f64::INFINITY };
            if icls == FpClass::Zero {
                retval.re = value;
                retval.im = x.im;
            } else {
                let (sinix, cosix);
                if x.im.abs() > f64::MIN_POSITIVE {
                    sinix = x.im.sin();
                    cosix = x.im.cos();
                } else {
                    sinix = x.im;
                    cosix = 1.0;
                }
                retval.re = f64::copysign(value, cosix);
                retval.im = f64::copysign(value, sinix);
            }
        } else if !x.re.is_sign_negative() {
            retval.re = f64::INFINITY;
            retval.im = x.im - x.im;
        } else {
            retval.re = 0.0;
            retval.im = f64::copysign(0.0, x.im);
        }
    } else {
        // Real part is NaN: NaN + iNaN unless the imaginary part is
        // zero (glibc s_cexp_template.c).
        retval.re = f64::NAN;
        if icls == FpClass::Zero {
            retval.im = x.im;
        } else {
            retval.im = f64::NAN;
        }
    }
    retval
}

// R: math/s_csqrt_template.c:__csqrt
pub fn csqrt(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut res = Complex::new(0.0, 0.0);
    if rcls == FpClass::Nan || rcls == FpClass::Infinite || icls == FpClass::Nan || icls == FpClass::Infinite {
        if icls == FpClass::Infinite {
            res.re = f64::INFINITY;
            res.im = x.im;
        } else if rcls == FpClass::Infinite {
            if x.re < 0.0 {
                res.re = if icls == FpClass::Nan { f64::NAN } else { 0.0 };
                res.im = f64::copysign(f64::INFINITY, x.im);
            } else {
                res.re = x.re;
                res.im = if icls == FpClass::Nan { f64::NAN } else { f64::copysign(0.0, x.im) };
            }
        } else {
            res.re = f64::NAN;
            res.im = f64::NAN;
        }
    } else if icls == FpClass::Zero {
        if x.re < 0.0 {
            res.re = 0.0;
            res.im = f64::copysign((-x.re).sqrt(), x.im);
        } else {
            res.re = x.re.sqrt().abs();
            res.im = f64::copysign(0.0, x.im);
        }
    } else if rcls == FpClass::Zero {
        let r = if x.im.abs() >= 2.0 * f64::MIN_POSITIVE {
            (0.5 * x.im.abs()).sqrt()
        } else {
            0.5 * (2.0 * x.im.abs()).sqrt()
        };
        res.re = r;
        res.im = f64::copysign(r, x.im);
    } else {
        let mut xre = x.re;
        let mut xim = x.im;
        let mut scale = 0i32;
        if xre.abs() > f64::MAX / 4.0 {
            scale = 1;
            xre = crate::nmath::ldexp(xre, -2 * scale);
            xim = crate::nmath::ldexp(xim, -2 * scale);
        } else if xim.abs() > f64::MAX / 4.0 {
            scale = 1;
            if xre.abs() >= 4.0 * f64::MIN_POSITIVE {
                xre = crate::nmath::ldexp(xre, -2 * scale);
            } else {
                xre = 0.0;
            }
            xim = crate::nmath::ldexp(xim, -2 * scale);
        } else if xre.abs() < 2.0 * f64::MIN_POSITIVE && xim.abs() < 2.0 * f64::MIN_POSITIVE {
            scale = -((f64::MANTISSA_DIGITS as i32 + 1) / 2);
            xre = crate::nmath::ldexp(xre, -2 * scale);
            xim = crate::nmath::ldexp(xim, -2 * scale);
        }
        let d = xre.hypot(xim);
        let (mut r, mut s);
        if xre > 0.0 {
            r = (0.5 * (d + xre)).sqrt();
            if scale == 1 && xim.abs() < 1.0 {
                // Avoid possible intermediate underflow.
                s = xim / r;
                r = crate::nmath::ldexp(r, scale);
                scale = 0;
            } else {
                s = 0.5 * (xim / r);
            }
        } else {
            s = (0.5 * (d - xre)).sqrt();
            if scale == 1 && xim.abs() < 1.0 {
                // Avoid possible intermediate underflow.
                r = (xim / s).abs();
                s = crate::nmath::ldexp(s, scale);
                scale = 0;
            } else {
                r = (0.5 * (xim / s)).abs();
            }
        }
        if scale != 0 {
            r = crate::nmath::ldexp(r, scale);
            s = crate::nmath::ldexp(s, scale);
        }
        res.re = r;
        res.im = f64::copysign(s, x.im);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(re: f64, im: f64) -> Complex {
        Complex::new(re, im)
    }

    fn bits(c: Complex) -> (u64, u64) {
        (c.re.to_bits(), c.im.to_bits())
    }

    #[test]
    fn mul_div_reference() {
        // From GNU R 4.5.0 (writeBin of the bits): Inf*1i is NaN+Infi —
        // the real part is the SSE default QNaN 0xfff8000000000000
        // (sign bit set), not 0x7ff8....
        assert_eq!(
            bits(cmul(z(f64::INFINITY, 0.0), z(0.0, 1.0))),
            bits(z(f64::from_bits(0xfff8000000000000), f64::INFINITY))
        );
        assert_eq!(bits(cmul(z(f64::INFINITY, 1.0), z(f64::INFINITY, 1.0))), bits(z(f64::INFINITY, f64::INFINITY)));
        // 1/(Inf+1i) is 0+0i; (1+2i)/(0+0i) is Inf+Infi.
        assert_eq!(bits(cdiv(z(1.0, 0.0), z(f64::INFINITY, 1.0))), bits(z(0.0, 0.0)));
        assert_eq!(bits(cdiv(z(1.0, 2.0), z(0.0, 0.0))), bits(z(f64::INFINITY, f64::INFINITY)));
        assert_eq!(bits(cdiv(z(1.0, 0.0), z(f64::NAN, f64::INFINITY))), bits(z(0.0, -0.0)));
    }

    #[test]
    fn pow_reference() {
        assert_eq!(bits(cpow(z(0.0, 0.0), z(2.0, 0.0))), bits(z(0.0, 0.0)));
        assert_eq!(bits(cpow(z(0.0, 0.0), z(-1.0, 0.0))), bits(z(f64::INFINITY, 0.0)));
        assert_eq!(bits(cpow(z(0.0, 0.0), z(0.0, 1.0))), bits(z(f64::NAN, f64::NAN)));
        assert_eq!(bits(cpow(z(0.0, 0.0), z(0.5, 0.0))), bits(z(0.0, 0.0)));
        assert_eq!(bits(cpow(z(1.0, 1.0), z(0.0, 0.0))), bits(z(1.0, 0.0)));
        // (-1+0i)^0.5 via cexp(0.5*clog(-1)) = cos(pi/2)+i sin(pi/2)
        let r = cpow(z(-1.0, 0.0), z(0.5, 0.0));
        assert_eq!(r.re.to_bits(), 6.123233995736766e-17f64.to_bits());
        assert_eq!(r.im.to_bits(), 1.0f64.to_bits());
        // (2+0i)^2 = 4+0i (integer-exponent path)
        assert_eq!(bits(cpow(z(2.0, 0.0), z(2.0, 0.0))), bits(z(4.0, 0.0)));
    }

    #[test]
    fn log_exp_sqrt_reference() {
        assert_eq!(clog(z(0.0, 1.0)), z(0.0, std::f64::consts::FRAC_PI_2));
        assert_eq!(clog(z(0.0, 0.0)).re, f64::NEG_INFINITY);
        assert_eq!(cexp(z(f64::INFINITY, 1.0)), z(f64::INFINITY, f64::INFINITY));
        assert_eq!(csqrt(z(0.0, 0.0)), z(0.0, 0.0));
        assert_eq!(csqrt(z(-1.0, 0.0)), z(0.0, 1.0));
        // sqrt(NA+0i): NA propagates through the hypot path as a NaN.
        assert!(csqrt(z(crate::value::NA_REAL, 0.0)).re.is_nan());
    }

    /// The kernels are bit-identical with the C library GNU R uses
    /// (libgcc's `__muldc3`/`__divdc3`, glibc's `clog`/`cexp`/`csqrt`/
    /// `cpow`) over a grid of special values and random bit patterns,
    /// recorded by `oracle.c` on x86-64 glibc.
    #[test]
    fn c_library_oracle() {
        use std::io::Read;
        let raw = include_bytes!("complex_oracle.txt.gz");
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s).unwrap();
        let n = s.lines().count();
        let mut checked = 0usize;
        for line in s.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() != 7 {
                panic!("bad line: {line}");
            }
            let hex = |h: &str| f64::from_bits(u64::from_str_radix(h, 16).unwrap());
            let (a, b) = (z(hex(t[1]), hex(t[2])), z(hex(t[3]), hex(t[4])));
            let want = (hex(t[5]), hex(t[6]));
            let got = match t[0] {
                "mul" => cmul(a, b),
                "div" => cdiv(a, b),
                "pow" => cpow(a, b),
                "log" => clog(a),
                "exp" => cexp(a),
                "sqrt" => csqrt(a),
                other => panic!("bad op {other}"),
            };
            assert_eq!(
                (got.re.to_bits(), got.im.to_bits()),
                (want.0.to_bits(), want.1.to_bits()),
                "{} ({}, {}) ({}, {})",
                t[0],
                t[1],
                t[2],
                t[3],
                t[4]
            );
            checked += 1;
        }
        assert!(checked == n && n > 50000, "{checked} of {n}");
    }
}
