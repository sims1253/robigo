//! The x87 microcode instructions `f2xm1`, `fyl2x` and `fyl2xp1`, which
//! glibc's x86_64 `expl`, `logl`, `log1pl` and `powl` execute.
//!
//! The hardware computes them to within one ulp but does not round them
//! correctly, and which results miss the correctly rounded value depends
//! on the processor: on the same inputs (`libm_matches_glibc` in the
//! property tests), glibc's `logl` is one ulp off for 1.04% of results on
//! an AMD Zen 3 and for 3.28% on one GitHub Actions runner type. GNU R's
//! own long-double results therefore differ in the last bit between
//! x86_64 machines, and no software version can match all of them.
//!
//! [`Exact`] gives the correctly rounded result, computed on a
//! 128-bit-significand core, which every x86_64 processor gets within an
//! ulp of. The ports in `libm.rs` and `pow.rs` are generic over
//! [`Microcode`], so that the property tests can run them on the
//! hardware instructions as well and require glibc's results bit for bit.

use super::{Class, LDouble, class, indefinite, infinity, is_nan, norm, pack, quiet, zero};

/// The three microcode instructions, as the ports call them: with the
/// x87's quiet-NaN and invalid-operand results for the operands the
/// instruction sees.
pub(crate) trait Microcode {
    /// `f2xm1`: 2^x − 1, |x| ≤ 1.
    fn f2xm1(x: &LDouble) -> LDouble;
    /// `fyl2x`: y·log2(x), y > 0 finite.
    fn fyl2x(x: &LDouble, y: &LDouble) -> LDouble;
    /// `fyl2xp1`: y·log2(1 + x), |x| < 1 − √½, y > 0 finite.
    fn fyl2xp1(x: &LDouble, y: &LDouble) -> LDouble;
}

/// The instructions correctly rounded: what the library uses.
pub(crate) struct Exact;

impl Microcode for Exact {
    fn f2xm1(x: &LDouble) -> LDouble {
        f2xm1(x)
    }
    fn fyl2x(x: &LDouble, y: &LDouble) -> LDouble {
        fyl2x(x, y)
    }
    fn fyl2xp1(x: &LDouble, y: &LDouble) -> LDouble {
        fyl2xp1(x, y)
    }
}

// ---------------------------------------------------------------------
// A 128-bit-significand float for the correctly rounded instructions.

/// value =(-1)^s · m · 2^e with m ∈ {0} ∪ [2^127, 2^128). Every
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

    /// `n · 2^e`, exact.
    fn from_int(n: u128, e: i32, s: bool) -> X128 {
        if n == 0 {
            return X128::ZERO;
        }
        let sh = n.leading_zeros();
        X128 { m: n << sh, e: e - sh as i32, s }
    }

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
        let rs = r << sh;
        let up = 2 * rs > (k as u128) || (2 * rs == (k as u128) && m & 1 != 0);
        let mut m = m + up as u128;
        let mut e = e;
        if m == 0 {
            m = 1u128 << 127;
            e += 1;
        }
        X128 { m, e, s: a.s }
    }

    /// a / b (b ≠ 0), nearest even: restoring long division, one
    /// quotient bit per step, the remainder deciding the rounding.
    fn div(a: X128, b: X128) -> X128 {
        debug_assert!(b.m != 0);
        if a.m == 0 {
            return X128::ZERO;
        }
        // value = (a.m / b.m)·2^(a.e − b.e); scale a.m so the ratio is in
        // [1, 2), keeping the 129th remainder bit in `hi`.
        let mut e = a.e - b.e - 127;
        let (mut hi, mut r) = (false, a.m);
        if a.m < b.m {
            hi = r >> 127 != 0;
            r <<= 1;
            e -= 1;
        }
        let mut q: u128 = 0;
        for _ in 0..128 {
            q <<= 1;
            if hi || r >= b.m {
                r = r.wrapping_sub(b.m); // hi·2^128 + r − b.m < b.m < 2^128
                q |= 1;
            }
            hi = r >> 127 != 0;
            r <<= 1;
        }
        // hi·2^128 + r is now twice the remainder: compare with b.m
        let up = hi || r > b.m || (r == b.m && q & 1 != 0);
        if up {
            q = q.wrapping_add(1);
            if q == 0 {
                return X128 { m: 1u128 << 127, e: e + 1, s: a.s ^ b.s };
            }
        }
        X128 { m: q, e, s: a.s ^ b.s }
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

/// 1/ln2 to 128 bits.
const RECIP_LN2: X128 = X128 { m: 0xb8aa_3b29_5c17_f0bb_be87_fed0_691d_3e89, e: -127, s: false };

/// ln(1 + t) for −0.3 < t < 0.42, as 2·atanh(t / (2 + t)): the series
/// in u² ≤ 0.03 gains five bits a term, and every term has the sign of
/// t, so the sum never cancels. Relative error a few units of 2^-128.
fn ln1p_x128(t: X128) -> X128 {
    if t.m == 0 {
        return t;
    }
    const TWO: X128 = X128 { m: 1u128 << 127, e: -126, s: false };
    let u = X128::div(t, X128::addsub(TWO, t, false));
    let u2 = X128::mul(u, u);
    let mut sum = u;
    let mut p = u; // u^k
    let mut k: u64 = 1;
    loop {
        k += 2;
        p = X128::mul(p, u2);
        let term = X128::div_small(p, k);
        if term.m == 0 || sum.e - term.e > 130 {
            break;
        }
        sum = X128::addsub(sum, term, false);
    }
    X128 { e: sum.e + 1, ..sum }
}

/// `fyl2x` for y > 0 finite: y·log2(x), correctly
/// rounded. log2(x) = k + log2(f) with f ∈ [√½, √2), so the sum never
/// cancels below |log2(f)|.
fn fyl2x(x: &LDouble, y: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Zero => return infinity(true), // divide by zero: −inf·sign(y)
        Class::Inf => return if s { indefinite() } else { *x },
        _ => {}
    }
    if s {
        return indefinite();
    }
    // x = m·2^(e2−63), m ∈ [2^63, 2^64); t = f − 1 exactly.
    let (m, e2) = norm(x);
    const SQRT2_M: u64 = 0xb504_f333_f9de_6484; // ⌊√2·2^63⌋
    let (t, k) = if m <= SQRT2_M {
        (X128::from_int((m - (1u64 << 63)) as u128, -63, false), e2)
    } else {
        (X128::from_int((1u128 << 64) - m as u128, -64, true), e2 + 1)
    };
    let log2f = X128::mul(ln1p_x128(t), RECIP_LN2);
    let log2x = X128::addsub(X128::from_int(k.unsigned_abs() as u128, 0, k < 0), log2f, false);
    X128::mul(log2x, X128::from_ld(y)).to_ld()
}

/// `fyl2xp1` for y > 0 finite: y·log2(1 + x), |x| < 1 − √½, correctly
/// rounded.
fn fyl2xp1(x: &LDouble, y: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal | Class::Inf => return indefinite(),
        Class::Zero => return zero(s),
        _ => {}
    }
    let log2 = X128::mul(ln1p_x128(X128::from_ld(x)), RECIP_LN2);
    X128::mul(log2, X128::from_ld(y)).to_ld()
}
