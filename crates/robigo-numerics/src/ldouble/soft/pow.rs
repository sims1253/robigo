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
//! are replaced by correctly-rounded software ([`f2xm1`], [`fyl2xp1`])
//! built on a 128-bit-significand core. The hardware versions are *not*
//! correctly rounded: measured against a `__float128` reference, 1.6-3.2%
//! of results (uniformly over the input space, at every exponent) differ
//! from the correctly rounded value by 1 ulp, and the error traces a
//! smooth deterministic curve — a rounded polynomial inside the
//! microcode. Bit-exact replication would mean reverse-engineering that
//! polynomial; short of that, this port agrees with glibc's `powl` bit
//! for bit everywhere glibc itself is correctly rounded, and within one
//! final-bit ulp elsewhere. The property tests measure the exact rate.

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
const fn ld(m: u64, e2: i32) -> LDouble {
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

const fn pow2(k: i32) -> LDouble {
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
const M_LOG2E: LDouble = ld(0xb8aa_3b29_5c17_f0bc, -63);
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

// ---------------------------------------------------------------------
// A 128-bit-significand float for the correctly-rounded f2xm1/fyl2xp1.

/// value = (-1)^s · m · 2^e with m ∈ {0} ∪ [2^127, 2^128). Every
/// operation rounds to nearest even at 128 bits (guard + sticky), so a
/// short chain keeps ~2^-110 relative error — far below what the 64-bit
/// final rounding can see.
#[derive(Clone, Copy)]
struct X128 {
    m: u128,
    e: i32,
    s: bool,
}

/// (hi, lo) with a·b = hi·2^128 + lo.
fn mul128(a: u128, b: u128) -> (u128, u128) {
    let (a1, a0) = (a >> 64, a & u64::MAX as u128);
    let (b1, b0) = (b >> 64, b & u64::MAX as u128);
    let a1b1 = a1 * b1;
    let a1b0 = a1 * b0;
    let a0b1 = a0 * b1;
    let a0b0 = a0 * b0;
    // mid = a1b0 + a0b1 reaches 2^129: its bit 128 lands 2^64 above hi's
    // low word (hi = a1b1 + mid>>64 + carry, all in units of 2^128).
    let (mid, mid_carry) = a1b0.overflowing_add(a0b1);
    let lo = a0b0.wrapping_add(mid << 64);
    let carry = (lo < a0b0) as u128 + (mid >> 64) + ((mid_carry as u128) << 64);
    (a1b1 + carry, lo)
}

impl X128 {
    const ZERO: X128 = X128 { m: 0, e: 0, s: false };

    fn from_ld(x: &LDouble) -> X128 {
        let (c, s) = class(x);
        debug_assert!(!is_nan(c) && c != Class::Inf && c != Class::Zero);
        let (m, e2) = norm(x);
        X128 { m: (m as u128) << 64, e: e2 - 127, s }
    }

    /// Round to the 80-bit format, nearest even (single rounding,
    /// denormals included — `pack` does exactly that for a u128).
    fn to_ld(&self) -> LDouble {
        if self.m == 0 {
            return zero(self.s);
        }
        pack(self.s, self.m, self.e, false)
    }

    fn mul(a: X128, b: X128) -> X128 {
        if a.m == 0 || b.m == 0 {
            return X128::ZERO;
        }
        // value = (hi·2^128 + lo)·2^(a.e + b.e - 256), hi ∈ [2^126, 2^128):
        // normalize the significand to [2^127, 2^128), keeping one guard
        // bit and a sticky from the low half.
        let (hi, lo) = mul128(a.m, b.m);
        let (mut m, e, g, st) = if hi >= 1u128 << 127 {
            // value = hi·2^128·2^(ae+be)
            (hi, a.e + b.e + 128, lo >> 127 != 0, lo & ((1u128 << 127) - 1) != 0)
        } else {
            // value = m·2^127·2^(ae+be) with m = (hi<<1)|(lo>>127)
            ((hi << 1) | (lo >> 127), a.e + b.e + 127, (lo >> 126) & 1 != 0, lo & ((1u128 << 126) - 1) != 0)
        };
        if g && (st || m & 1 != 0) {
            m = m.wrapping_add(1);
            let mut e = e;
            if m == 0 {
                m = 1u128 << 127;
                e += 1;
            }
            return X128 { m, e, s: a.s ^ b.s };
        }
        X128 { m, e, s: a.s ^ b.s }
    }

    /// Division by a small integer, nearest even at the final
    /// (normalized) 128-bit grid.
    fn div_small(a: X128, k: u64) -> X128 {
        if a.m == 0 || k == 1 {
            return a;
        }
        let q = a.m / k as u128;
        let r = a.m % k as u128;
        let sh = q.leading_zeros();
        let m = q << sh;
        let e = a.e - sh as i32;
        // fraction in units of the new lsb: (r/k)·2^sh
        let rs = (r as u128) << sh;
        let up = 2 * rs > (k as u128) || (2 * rs == (k as u128) && m & 1 != 0);
        let mut m = m + up as u128;
        let mut e = e;
        if m == 0 {
            m = 1u128 << 127;
            e += 1;
        }
        X128 { m, e, s: a.s }
    }

    fn addsub(a: X128, b: X128, sub: bool) -> X128 {
        let b = X128 { s: b.s ^ sub, ..b };
        if a.m == 0 {
            return b;
        }
        if b.m == 0 {
            return a;
        }
        if a.s == b.s {
            let (m, e) = mag_add(a.m, a.e, b.m, b.e);
            return X128 { m, e, s: a.s };
        }
        // Opposite signs: subtract the smaller magnitude from the larger.
        let ((ma, ea, sa), (mb, eb, sb)) = if (a.e, a.m) >= (b.e, b.m) {
            ((a.m, a.e, a.s), (b.m, b.e, b.s))
        } else {
            ((b.m, b.e, b.s), (a.m, a.e, a.s))
        };
        let d = ea - eb;
        debug_assert!(d >= 0);
        if d > 129 {
            return X128 { m: ma, e: ea, s: sa };
        }
        let (keep, g, st) = shift_keep(mb, d);
        // true = ma − keep − g/2 − stf, stf ∈ (0, ½) if st
        if ma == keep && (g || st) {
            // the difference is entirely below ma's lsb: |true| < 1 lsb
            if g && st {
                // rounds to one lsb with the subtrahend's sign
                return X128 { m: 1, e: ea - 127, s: sb };
            }
            return X128::ZERO; // below or at the half: 0
        }
        let mut m = ma - keep; // ≥ 1 unless ma == keep (handled above)
        if g && st {
            m -= 1; // closer to the integer below
        } else if g && !st && m & 1 != 0 {
            m -= 1; // exact halfway: round to even
        }
        if m == 0 {
            return X128::ZERO;
        }
        let sh = m.leading_zeros() as i32;
        X128 { m: m << sh, e: ea - sh, s: sa }
    }
}

/// `mb` shifted down by `d` (0 ≤ d ≤ 129), split into (kept, guard, sticky).
fn shift_keep(mb: u128, d: i32) -> (u128, bool, bool) {
    if d == 0 {
        (mb, false, false)
    } else if d <= 127 {
        let d = d as u32;
        (mb >> d, (mb >> (d - 1)) & 1 != 0, mb & ((1u128 << (d - 1)) - 1) != 0)
    } else if d == 128 {
        (0, mb >> 127 != 0, mb & ((1u128 << 127) - 1) != 0)
    } else {
        (0, false, mb != 0)
    }
}

/// |a| + |b| for normalized mantissas, nearest even at the result grid.
fn mag_add(ma: u128, ea: i32, mb: u128, eb: i32) -> (u128, i32) {
    let (ma, ea, mb, eb) = if ea >= eb { (ma, ea, mb, eb) } else { (mb, eb, ma, ea) };
    let d = ea - eb;
    if d > 129 {
        return (ma, ea); // b is below half an ulp of a
    }
    let (keep, g, st) = shift_keep(mb, d);
    let s = ma.wrapping_add(keep);
    let (m, e, up) = if s < ma {
        // carried into bit 128: renormalize by one; the dropped lsb of s
        // becomes the half, g/st drop another factor of two.
        let m = (1u128 << 127) | (s >> 1);
        let e = ea + 1;
        let up = if s & 1 != 0 { g || st } else { false };
        // exact halfway (s odd, no g/st): round to even
        let up = up || (s & 1 != 0 && !g && !st && m & 1 != 0);
        (m, e, up)
    } else {
        let up = g && (st || ma.wrapping_add(keep) & 1 != 0);
        (s, ea, up)
    };
    if up {
        let mut m = m.wrapping_add(1);
        let mut e = e;
        if m == 0 {
            m = 1u128 << 127;
            e += 1;
        }
        (m, e)
    } else {
        (m, e)
    }
}

/// `f2xm1` (correctly rounded): 2^x − 1 for |x| ≤ 1, via the Taylor
/// series of expm1 in t = x·ln2 (ln2 to 128 bits).
fn f2xm1(x: &LDouble) -> LDouble {
    let (c, _) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Zero => return zero(false),
        Class::Inf => return indefinite(), // out of the instruction domain
        _ => {}
    }
    const LN2: X128 = X128 { m: 0xb172_17f7_d1cf_79ab_c9e3_b398_03f2_f6af, e: -128, s: false };
    let t = X128::mul(X128::from_ld(x), LN2);
    let mut sum = t;
    let mut term = t;
    let mut k: u64 = 1;
    loop {
        k += 1;
        term = X128::mul(term, t);
        term = X128::div_small(term, k);
        sum = X128::addsub(sum, term, false);
        if term.m == 0 || sum.e - term.e > 85 || k >= 45 {
            break;
        }
    }
    sum.to_ld()
}

/// `fyl2xp1` with the second operand 1.0 (correctly rounded): log2(1+x),
/// |x| < 1 − 1/√2, via the Mercator series in x and 1/ln2 to 128 bits.
fn fyl2xp1(x: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Zero => return zero(s),
        Class::Inf => return indefinite(),
        _ => {}
    }
    const RECIP_LN2: X128 = X128 { m: 0xb8aa_3b29_5c17_f0bb_be87_fed0_691d_3e89, e: -127, s: false };
    let xx = X128::from_ld(x);
    let mut sum = xx; // k = 1 term
    let mut p = xx; // x^k
    let mut k: u64 = 1;
    loop {
        k += 1;
        p = X128::mul(p, xx);
        let mut term = p;
        // log(1+x) = Σ (−1)^{k+1} x^k / k: for x < 0 every term is
        // negative; for x > 0 the even ones are.
        term.s = s || (k % 2 == 0);
        term = X128::div_small(term, k);
        sum = X128::addsub(sum, term, false);
        if p.m == 0 || sum.e - p.e > 92 || k >= 90 {
            break;
        }
    }
    X128::mul(sum, RECIP_LN2).to_ld()
}

/// `fscale`: x · 2^trunc(i), `i` integer-valued.
fn fscale(x: &LDouble, i: &LDouble) -> LDouble {
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
fn helper(x0: &LDouble, y: &LDouble) -> LDouble {
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
    let log2_1p_w_lo = fyl2xp1(&div(&w_lo, &add(&LDouble::ONE, &w_hi)));
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

    let mut res = add(&f2xm1(&log2_res_frac), &LDouble::ONE);
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
                return helper(x, y);
            }
            // Exact binary powering (e_powl.S labels 4-6): the asm keeps
            // the accumulator |acc| and a base that preserves x's sign,
            // squaring the base as base·|base| each iteration.
            let ri = rint(y);
            let n = to_u64_trunc(&fabs(&ri)); // |y| ∈ {1, 2, 3}
            let base0 = if sy { div(&LDouble::ONE, x) } else { *x };
            let mut base = if n % 2 == 0 { fabs(&base0) } else { base0 };
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
            return helper(x, &if sy { PM79 } else { with_sign(&PM79, true) });
        }
        return helper(x, y);
    }
    // |y| ≥ 2^63: saturate at 2^78, sign preserved.
    if cmp(&absy, &P78) < 0 {
        helper(x, y)
    } else if sy {
        helper(x, &with_sign(&P78, true))
    } else {
        helper(x, &P78)
    }
}
