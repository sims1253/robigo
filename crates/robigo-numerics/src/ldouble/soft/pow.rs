//! `powl` as glibc 2.41 implements it on x86_64, ported operation by
//! operation.
//!
//! - R: sysdeps/x86_64/fpu/e_powl.S:__ieee754_powl — the special cases,
//!   exact binary powering for integer |y| < 4, saturation of |y| to
//!   [2^-79, 2^78], and the call into the helper.
//! - R: sysdeps/x86/fpu/powl_helper.c:__powl_helper — the table + power
//!   series algorithm for real exponents, with every long-double
//!   operation exactly rounded at 64 bits, as on x87.
//!
//! The helper's two x87 microcode instructions, `f2xm1` and `fyl2xp1`,
//! come from a [`Microcode`]: correctly rounded ([`Exact`]) in the
//! library, the hardware's in the property tests. The hardware versions
//! are *not* correctly rounded: measured against a `__float128`
//! reference, 1.6-3.2% of results (uniformly over the input space, at
//! every exponent) differ from the correctly rounded value by 1 ulp, and
//! the error traces a smooth deterministic curve — a rounded polynomial
//! inside the microcode, which differs between processors (see
//! `microcode.rs`). So this port agrees with glibc's `powl` bit for bit
//! everywhere glibc itself is correctly rounded, and within one
//! final-bit ulp elsewhere; on the hardware's microcode it agrees
//! everywhere.

use super::microcode::{Exact, Microcode};
use super::{
    BIAS, Class, LDouble, add, class, cmp, div, from_i64, indefinite, infinity, is_nan, join, make, mul, norm, pack,
    quiet, rint, split, sub, with_sign, zero,
};

fn fabs(x: &LDouble) -> LDouble {
    with_sign(x, false)
}

// ---------------------------------------------------------------------
// Constants: exact x87 encodings, generated from the C literals by
// rounding their exact values to a 64-bit significand, nearest-even.

/// `ld(m, e2)`: `m` (integer bit set) times `2^e2`.
pub(super) const fn ld(m: u64, e2: i32) -> LDouble {
    let se = (e2 + BIAS + 63) as u16;
    LDouble([
        m as u8,
        (m >> 8) as u8,
        (m >> 16) as u8,
        (m >> 24) as u8,
        (m >> 32) as u8,
        (m >> 40) as u8,
        (m >> 48) as u8,
        (m >> 56) as u8,
        se as u8,
        (se >> 8) as u8,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
}

const fn negc(x: LDouble) -> LDouble {
    let mut b = x.0;
    b[9] |= 0x80;
    LDouble(b)
}

pub(super) const fn pow2(k: i32) -> LDouble {
    ld(1u64 << 63, k - 63)
}

/// High and low parts of -log(k/16), k = 12..24 (glibc powl_helper.c).
#[rustfmt::skip]
const POWL_LOG_TABLE: [LDouble; 26] = [
    ld(0x934b1089a6dc93c2, -65),            // k=12 hi  0x4.9a58844d36e49e1p-4
    negc(ld(0x82913127eaac7aba, -131)),      // k=12 lo -0x1.0522624fd558f574p-68
    ld(0xd49f69e456cf1b79, -66),            // k=13 hi
    ld(0xbea77a5c80dccdcf, -131),           // k=13 lo
    ld(0x88bc74113f23def2, -66),            // k=14 hi
    negc(ld(0xc74be038d217e1c4, -131)),     // k=14 lo
    ld(0x842cc5acf1d03445, -67),            // k=15 hi
    ld(0xff66fd40cdcb04bf, -134),           // k=15 lo
    LDouble::ZERO,                           // k=16: log(1) = 0
    LDouble::ZERO,
    negc(ld(0xf85186008b15330c, -68)),      // k=17 hi -0xf.85186008b15330cp-8
    ld(0xcda3a4453343b396, -135),           // k=17 lo
    negc(ld(0xf1383b7157972f4f, -67)),      // k=18 hi
    negc(ld(0xa87ffe1fe9e155dc, -132)),     // k=18 lo
    negc(ld(0xaff983853c9e9e44, -66)),      // k=19 hi
    ld(0xc1df5f8dedc45019, -131),           // k=19 lo
    negc(ld(0xe47fbe3cd4d10d61, -66)),      // k=20 hi
    negc(ld(0xbb03de5ff734495c, -132)),     // k=20 lo
    negc(ld(0x8b3ae55d5d30701d, -65)),      // k=21 hi
    ld(0xce0aa3be4747dc10, -132),           // k=21 lo
    negc(ld(0xa30c5e10e2f613e8, -65)),      // k=22 hi
    negc(ld(0xb7b37b33c734415e, -130)),     // k=22 lo
    negc(ld(0xb9cebfb5de8034e7, -65)),      // k=23 hi
    negc(ld(0x9314feb4fbde5aae, -132)),     // k=23 lo
    negc(ld(0xcf991f65fcc25f96, -65)),      // k=24 hi
    ld(0x9728990bfadde7e6, -130),           // k=24 lo
];

/// log2(e) high part: 0x1.71547652p+0 (exact).
const LOG2E_HI: LDouble = ld(0xb8aa_3b29_0000_0000, -63);
/// log2(e) low part: 0xb.82fe1777d0ffda1p-36 (rounded to 64 bits).
const LOG2E_LO: LDouble = ld(0xb82f_e177_7d0f_fda1, -96);
/// M_LOG2El (glibc's decimal literal, correctly rounded).
pub(super) const M_LOG2E: LDouble = ld(0xb8aa_3b29_5c17_f0bc, -63);
/// The literal 0x0.aaaaaaaaaaaaaaaap0: 2/3 rounded down (exact).
const TWO_THIRDS: LDouble = ld(0xaaaa_aaaa_aaaa_aaaa, -64);
/// 1/3 high: 0x0.5555p0 (exact).
const THIRD_HI: LDouble = ld(0xaaaa_0000_0000_0000, -65);
/// 1/3 low: 0x0.5555555555555555p-16 (exact).
const THIRD_LO: LDouble = ld(0xaaaa_aaaa_aaaa_aaaa, -81);
const HALF: LDouble = pow2(-1);
const SIXTEEN: LDouble = pow2(4);
const P63: LDouble = pow2(63);
const P64: LDouble = pow2(64);
const P78: LDouble = pow2(78);
const PM79: LDouble = pow2(-79);

/// `fscale`: x · 2^trunc(i), `i` integer-valued.
pub(super) fn fscale(x: &LDouble, i: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Zero => return zero(s),
        Class::Inf => return *x,
        _ => {}
    }
    let (ci, _) = class(i);
    let k = if ci == Class::Zero {
        0
    } else {
        let (mi, ei) = norm(i);
        let (_, si) = class(i);
        debug_assert!(ei <= 93);
        // |i| can reach ~2^92; when |i| ≥ 2^63 the scale saturates (the
        // result is ±inf or ±0 for any such k, and the caller zeroed
        // the fraction at |int| > 16500).
        if ei < 0 {
            0
        } else if ei >= 63 {
            if si { -17000 } else { 17000 }
        } else {
            (((mi >> (63 - ei)) as i64 as i128) * if si { -1 } else { 1 }).clamp(-17000, 17000) as i32
        }
    };
    let (m, e2) = norm(x);
    pack(s, (m as u128) << 64, e2 - 127 + k, false)
}

/// C99 `roundl`: round half away from zero.
fn roundl_ld(x: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Inf | Class::Zero => return *x,
        _ => {}
    }
    let (m, e2) = norm(x);
    if e2 >= 64 {
        return *x; // already an integer
    }
    if e2 < 0 {
        // |x| < 1: round half away from zero: [0.5, 1) → ±1, else ±0.
        return if e2 == -1 { with_sign(&pow2(0), s) } else { zero(s) };
    }
    let sh = (63 - e2) as u32;
    let i = if sh == 0 {
        m // |x| ∈ [2^63, 2^64): already an integer
    } else {
        let frac = m & ((1u64 << sh) - 1);
        let i = m >> sh;
        if frac >= 1u64 << (sh - 1) {
            i + 1 // ties round away from zero
        } else {
            i
        }
    };
    let l = i.leading_zeros() as i32;
    make(s, (63 - l + BIAS) as u16, i << l)
}

/// `(unsigned long long) x` for 0 < x < 2^64 (C truncation).
fn to_u64_trunc(x: &LDouble) -> u64 {
    let (m, e2) = norm(x);
    if e2 < 0 { 0 } else { m >> (63 - e2) }
}

fn to_i32_trunc(x: &LDouble) -> i32 {
    to_u64_trunc(x) as i32
}

/// `x mod 2^63` for |x| < 2^64 (the exact `fprem` the asm performs).
fn fprem_p63(x: &LDouble) -> LDouble {
    if cmp(&fabs(x), &P63) < 0 {
        return *x;
    }
    let (_, s) = class(x);
    sub(x, &with_sign(&P63, s)) // |x| ∈ [2^63, 2^64): subtract ±2^63 once
}

/// `(long double) v`, exact.
fn from_u64(v: u64) -> LDouble {
    if v >> 63 == 0 { from_i64(v as i64) } else { add(&from_i64((v & !(1u64 << 63)) as i64), &P63) }
}

/// R: powl_helper.c:__powl_helper. x finite nonzero, 2^-79 ≤ |y| ≤ 2^78.
fn helper<M: Microcode>(x0: &LDouble, y: &LDouble) -> LDouble {
    let mut x = *x0;
    let negate;
    if cmp(&x, &LDouble::ZERO) < 0 {
        let absy = fabs(y);
        if cmp(&absy, &P64) >= 0 {
            negate = false;
        } else {
            let yll = to_u64_trunc(&absy);
            if from_u64(yll) != absy {
                // expl(y·logl(x)): glibc's result is the negative real
                // indefinite qNaN (measured: ffff:c000000000000000)
                return make(true, 0x7fff, 0xc000_0000_0000_0000);
            }
            negate = yll & 1 == 1;
        }
        x = fabs(&x);
    } else {
        negate = false;
    }

    // frexpl
    let (m, e2) = norm(&x);
    let mut x_frac = make(false, (BIAS - 1) as u16, m); // m·2^-64 ∈ [0.5, 1)
    let mut x_int_exponent = e2 + 1;
    if cmp(&x_frac, &TWO_THIRDS) <= 0 {
        x_frac = add(&x_frac, &x_frac);
        x_int_exponent -= 1;
    }

    let k = to_i32_trunc(&add(&div(&SIXTEEN, &x_frac), &HALF));
    debug_assert!((12..=24).contains(&k), "k={k}");
    let mut log_x_frac_hi = POWL_LOG_TABLE[(2 * k - 24) as usize];
    let mut log_x_frac_lo = POWL_LOG_TABLE[(2 * k - 23) as usize];
    let x_frac_low;
    if k == 16 {
        x_frac_low = zero(false);
    } else {
        let (se, mf) = split(&x_frac);
        let masked = join(se, mf & !0x1f); // i1 &= 0xffffffe0
        x_frac_low = div(&sub(&x_frac, &masked), &masked);
        x_frac = mul(&masked, &div(&from_i64(k as i64), &SIXTEEN));
    }

    // log(1 + w) for w ∈ (−1/25, 1/25) via the series in w_hi, with the
    // compensated acc_split accumulation, exactly as in the C. w_hi
    // keeps 16 significand bits (i0 &= 0xffff0000, i1 = 0) so that the
    // powers of w_hi are exact products.
    let w = sub(&x_frac, &LDouble::ONE);
    let (se_w, mw) = split(&w);
    let w_hi = join(se_w, mw & 0xffff_0000_0000_0000);
    let w_lo = sub(&w, &w_hi);
    let neg_w_hi = with_sign(&w_hi, !is_neg(&w_hi));
    let mut wp = w_hi;
    acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, wp);
    wp = mul(&wp, &neg_w_hi);
    acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, div(&wp, &pow2(1)));
    wp = mul(&wp, &neg_w_hi);
    acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, mul(&wp, &THIRD_HI));
    acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, mul(&wp, &THIRD_LO));
    wp = mul(&wp, &neg_w_hi);
    acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, div(&wp, &from_i64(4)));
    for i in 5..=17 {
        wp = mul(&wp, &neg_w_hi);
        acc_split(&mut log_x_frac_hi, &mut log_x_frac_lo, div(&wp, &from_i64(i)));
    }

    // Convert to base 2; the 32-bit splits make the first products exact.
    let (se_h, mh) = split(&log_x_frac_hi);
    let hi32 = join(se_h, mh & 0xffff_ffff_0000_0000); // i1 = 0
    let lo64 = add(&sub(&log_x_frac_hi, &hi32), &log_x_frac_lo);
    let l2f_hi1 = mul(&hi32, &LOG2E_HI);
    let l2f_lo1 = add(&mul(&lo64, &LOG2E_HI), &mul(&log_x_frac_hi, &LOG2E_LO));
    let mut log2_x_frac_hi = add(&l2f_hi1, &l2f_lo1);
    let mut log2_x_frac_lo = add(&sub(&l2f_hi1, &log2_x_frac_hi), &l2f_lo1);

    // Corrections: log2(1 + w_lo/(1+w_hi)) and the masked-off x bits.
    let log2_1p_w_lo = M::fyl2xp1(&div(&w_lo, &add(&LDouble::ONE, &w_hi)), &LDouble::ONE);
    acc_split(&mut log2_x_frac_hi, &mut log2_x_frac_lo, log2_1p_w_lo);
    acc_split(&mut log2_x_frac_hi, &mut log2_x_frac_lo, mul(&x_frac_low, &M_LOG2E));

    let xint = from_i64(x_int_exponent as i64);
    let log2_x_hi = add(&xint, &log2_x_frac_hi);
    let log2_x_lo = add(&add(&sub(&xint, &log2_x_hi), &log2_x_frac_hi), &log2_x_frac_lo);

    let (se_h2, mh2) = split(&log2_x_hi);
    let hi32b = join(se_h2, mh2 & 0xffff_ffff_0000_0000);
    let lo64b = add(&sub(&log2_x_hi, &hi32b), &log2_x_lo);
    let (se_y, my) = split(y);
    let y_hi32 = join(se_y, my & 0xffff_ffff_0000_0000);
    let y_lo32 = sub(y, &y_hi32);
    let log2_res_hi = mul(&hi32b, &y_hi32);
    let log2_res_lo = add(&mul(&hi32b, &y_lo32), &mul(&lo64b, y));
    let log2_res_int = roundl_ld(&log2_res_hi);
    let mut log2_res_frac = add(&sub(&log2_res_hi, &log2_res_int), &log2_res_lo);
    if cmp(&fabs(&log2_res_int), &from_i64(16500)) > 0 {
        log2_res_frac = zero(false);
    }

    let mut res = add(&M::f2xm1(&log2_res_frac), &LDouble::ONE);
    if negate {
        res = with_sign(&res, !is_neg(&res));
    }
    fscale(&res, &log2_res_int)
}

fn is_neg(x: &LDouble) -> bool {
    split(x).0 & 0x8000 != 0
}

/// R: acc_split from powl_helper.c: add `x` into the (hi, lo) pair.
fn acc_split(hi: &mut LDouble, lo: &mut LDouble, x: LDouble) {
    let (h, l) = (*hi, *lo);
    let thi = add(&h, &x);
    let tlo = add(&add(&sub(&h, &thi), &x), &l);
    *hi = add(&thi, &tlo);
    *lo = add(&sub(&thi, hi), &tlo);
}

/// Whether finite nonzero `y` is an odd integer, as e_powl.S decides:
/// |y| ≥ 2^64 → even; else y mod 2^63 must be an integer with its low
/// bit set.
fn y_is_odd_integer(y: &LDouble) -> bool {
    if cmp(&fabs(y), &P64) >= 0 {
        return false;
    }
    let r = fprem_p63(y);
    let ri = rint(&r);
    if cmp(&ri, &r) != 0 {
        return false;
    }
    let (c, _) = class(&ri);
    if c == Class::Zero {
        return false;
    }
    let (m, e2) = norm(&ri);
    if e2 >= 63 {
        return false; // multiple of a power of two ≥ 2: even
    }
    (m >> (63 - e2)) & 1 == 1
}

/// R: sysdeps/x86_64/fpu/e_powl.S:__ieee754_powl.
pub fn powl(x: &LDouble, y: &LDouble) -> LDouble {
    powl_with::<Exact>(x, y)
}

/// [`powl`] on the microcode `M`.
pub(crate) fn powl_with<M: Microcode>(x: &LDouble, y: &LDouble) -> LDouble {
    let (cy, sy) = class(y);
    // y == ±0 → 1, unless x is a signaling NaN (or an unsupported
    // encoding), which yields the quieted x.
    if cy == Class::Zero {
        let (cx, _) = class(x);
        if is_nan(cx) {
            let (_, mx) = split(x);
            if mx & 0x4000_0000_0000_0000 == 0 {
                return add(x, x);
            }
        }
        return LDouble::ONE;
    }
    // y == ±inf
    if cy == Class::Inf {
        let (cx, _) = class(x);
        if is_nan(cx) {
            return add(x, x);
        }
        let absx = fabs(x);
        if cmp(&absx, &LDouble::ONE) == 0 {
            return LDouble::ONE;
        }
        // (|x| < 1) xor sign(y): +0 when true, +inf when false
        return if (cmp(&absx, &LDouble::ONE) < 0) != sy { zero(false) } else { infinity(false) };
    }
    // y NaN: pow(1, qNaN) = 1 (annex F); anything else → NaN via x + y
    if is_nan(cy) {
        if cmp(x, &LDouble::ONE) == 0 {
            let (_, my) = split(y);
            if my & 0x4000_0000_0000_0000 != 0 {
                return LDouble::ONE;
            }
        }
        return add(x, y);
    }

    let (cx, sx) = class(x);
    // x == ±0: the sign only survives for odd integer y < 0 (−inf/−0)
    if cx == Class::Zero {
        let odd = sx && y_is_odd_integer(y);
        return if sy {
            if odd { infinity(true) } else { infinity(false) }
        } else if odd {
            zero(true)
        } else {
            zero(false)
        };
    }
    // x == ±inf
    if cx == Class::Inf {
        if !sx {
            return if sy { zero(false) } else { infinity(false) };
        }
        if y_is_odd_integer(y) {
            return if sy { zero(true) } else { infinity(true) };
        }
        return if sy { zero(false) } else { infinity(false) };
    }
    // x NaN
    if is_nan(cx) {
        return add(x, y);
    }

    // Both finite and nonzero.
    let absy = fabs(y);
    if cmp(&absy, &P63) < 0 {
        if cmp(&rint(y), y) == 0 {
            // integer y with |y| < 2^63
            if cmp(&absy, &pow2(2)) >= 0 {
                return helper::<M>(x, y);
            }
            // Exact binary powering (e_powl.S labels 4-6): the asm keeps
            // the accumulator |acc| and a base that preserves x's sign,
            // squaring the base as base·|base| each iteration.
            let ri = rint(y);
            let n = to_u64_trunc(&fabs(&ri)); // |y| ∈ {1, 2, 3}
            let base0 = if sy { div(&LDouble::ONE, x) } else { *x };
            let mut base = if n.is_multiple_of(2) { fabs(&base0) } else { base0 };
            let mut acc = LDouble::ONE;
            let mut nn = n;
            while nn != 0 {
                if nn & 1 == 1 {
                    acc = mul(&fabs(&acc), &base);
                }
                base = mul(&base, &fabs(&base));
                nn >>= 1;
            }
            return acc;
        }
        if cmp(&absy, &PM79) < 0 {
            // Saturate to 2^-79. The asm's polarity (jnz on the sign
            // flag, unlike the 2^78 block): y > 0 → −2^-79, y < 0 → +2^-79.
            return helper::<M>(x, &if sy { PM79 } else { with_sign(&PM79, true) });
        }
        return helper::<M>(x, y);
    }
    // |y| ≥ 2^63: saturate at 2^78, sign preserved.
    if cmp(&absy, &P78) < 0 {
        helper::<M>(x, y)
    } else if sy {
        helper::<M>(x, &with_sign(&P78, true))
    } else {
        helper::<M>(x, &P78)
    }
}
