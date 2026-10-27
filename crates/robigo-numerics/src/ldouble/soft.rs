//! Software implementation of the x87 80-bit extended-precision format
//! (`long double` on x86_64 GNU systems): a 64-bit significand with an
//! explicit integer bit, a 16-bit biased exponent (bias 16383), and
//! round-to-nearest-even at 64 bits, including denormals, signed zeros,
//! infinities and NaN payloads (R's `NA_real_` payload 1954 survives every
//! operation, as it does on the FPU).
//!
//! This module provides the `imp` interface of [`super`] on targets
//! without x87 (aarch64, wasm32, ...), so exact-mode numerics never fall
//! back to `f64`. On x86_64 it is compiled too and checked against the
//! hardware by the property tests: every operation must agree with x87
//! bit for bit.
//!
//! Value semantics follow the x87 description in the Intel SDM; `powl`
//! (in [`powl`]) ports glibc 2.41's x86_64 `powl`.

use super::LDouble;

/// Exponent bias of the 80-bit format.
pub const BIAS: i32 = 16383;
/// Smallest normal exponent (unbiased).
const E_MIN: i32 = -16382;
/// Largest normal exponent (unbiased).
const E_MAX: i32 = 16383;
/// Denormal grid: a denormal's mantissa field counts units of 2^-16445.
const DENORM_UNIT_EXP: i32 = -16445;

// ---------------------------------------------------------------------
// Encoding

#[inline]
fn split(x: &LDouble) -> (u16, u64) {
    (u16::from_le_bytes([x.0[8], x.0[9]]), u64::from_le_bytes(x.0[..8].try_into().unwrap()))
}

#[inline]
fn join(se: u16, m: u64) -> LDouble {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&m.to_le_bytes());
    b[8..10].copy_from_slice(&se.to_le_bytes());
    LDouble(b)
}

#[inline]
fn make(s: bool, e: u16, m: u64) -> LDouble {
    join(e | if s { 0x8000 } else { 0 }, m)
}

/// The "real indefinite" QNaN produced by invalid operations.
#[inline]
fn indefinite() -> LDouble {
    join(0xffff, 0xc000_0000_0000_0000)
}

#[inline]
fn infinity(s: bool) -> LDouble {
    make(s, 0x7fff, 0x8000_0000_0000_0000)
}

#[inline]
fn zero(s: bool) -> LDouble {
    make(s, 0, 0)
}

// ---------------------------------------------------------------------
// Classification

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    SNan,
    QNan,
    /// Exponent all ones without the integer bit: unsupported on x87;
    /// loading it yields the real indefinite QNaN.
    PseudoNan,
    Inf,
    Zero,
    Denormal,
    Normal,
    /// A significand without the integer bit in the normal exponent
    /// range: also unsupported, also indefinite on load.
    Unnormal,
}

/// The x87 `fxam` class of a value and its sign.
#[inline]
pub fn class(x: &LDouble) -> (Class, bool) {
    let (se, m) = split(x);
    let s = se & 0x8000 != 0;
    let e = se & 0x7fff;
    let c = if e == 0x7fff {
        if m == 0x8000_0000_0000_0000 {
            Class::Inf
        } else if m & 0x8000_0000_0000_0000 == 0 {
            Class::PseudoNan
        } else if m & 0x4000_0000_0000_0000 != 0 {
            Class::QNan
        } else {
            Class::SNan
        }
    } else if e == 0 {
        if m == 0 { Class::Zero } else { Class::Denormal }
    } else if m & 0x8000_0000_0000_0000 != 0 {
        Class::Normal
    } else {
        Class::Unnormal
    };
    (c, s)
}

#[inline]
fn is_nan(c: Class) -> bool {
    matches!(c, Class::QNan | Class::SNan | Class::PseudoNan | Class::Unnormal)
}

/// The canonical encoding of a value: pseudo-denormals (exponent 0 with
/// the integer bit set) are re-encoded as the normal with the same
/// value, as the FPU does when it produces or passes them through.
fn canon_enc(x: &LDouble) -> LDouble {
    let (se, m) = split(x);
    if (se & 0x7fff) == 0 && m & 0x8000_0000_0000_0000 != 0 {
        let (m2, e2) = norm(x);
        return make(se & 0x8000 != 0, (e2 + BIAS) as u16, m2);
    }
    *x
}

/// `x` with its sign set to `s`.
fn with_sign(x: &LDouble, s: bool) -> LDouble {
    let (se, m) = split(x);
    join((se & 0x7fff) | if s { 0x8000 } else { 0 }, m)
}

/// Quiet a NaN (set bit 62), as x87 arithmetic does with sNaN operands.
#[inline]
fn quiet(x: &LDouble) -> LDouble {
    let (se, m) = split(x);
    join(se, m | 0x4000_0000_0000_0000)
}

/// Unsupported encodings load as the real indefinite QNaN (x87 raises
/// invalid on `fld` of them and uses that value).
#[inline]
fn canon(x: &LDouble) -> LDouble {
    match class(x).0 {
        Class::PseudoNan | Class::Unnormal => indefinite(),
        _ => *x,
    }
}

/// The NaN x87 arithmetic returns for a two-operand instruction: any
/// unsupported encoding (pseudo-NaN, unnormal) makes the result the real
/// indefinite QNaN; otherwise the NaN with the larger significand wins
/// (measured on the hardware; position and signaling do not matter),
/// ties to the positive one; signaling NaNs are quieted.
fn pick_nan(a: &LDouble, b: &LDouble) -> LDouble {
    let (ca, _) = class(a);
    let (cb, _) = class(b);
    if matches!(ca, Class::PseudoNan | Class::Unnormal) || matches!(cb, Class::PseudoNan | Class::Unnormal) {
        return indefinite();
    }
    let (sa, ma) = split(a);
    let (sb, mb) = split(b);
    let na = matches!(ca, Class::QNan | Class::SNan);
    let nb = matches!(cb, Class::QNan | Class::SNan);
    let w = match (na, nb) {
        (true, true) => {
            if ma > mb || ma == mb && (sa & 0x8000) <= (sb & 0x8000) {
                *a
            } else {
                *b
            }
        }
        (true, false) => *a,
        (false, true) => *b,
        (false, false) => indefinite(),
    };
    quiet(&w)
}

// ---------------------------------------------------------------------
// Normalized decomposition: value = m * 2^(e2 - 63), m in [2^63, 2^64).
// Denormals are normalized (pseudo-denormals carry the value of their
// encoding, as on x87).

#[inline]
fn norm(x: &LDouble) -> (u64, i32) {
    let (se, m) = split(x);
    let e = (se & 0x7fff) as i32;
    debug_assert!(e != 0x7fff && m != 0 && m & 0x8000_0000_0000_0000 != 0 || e == 0 && m != 0);
    if e != 0 {
        (m, e - BIAS)
    } else {
        let l = m.leading_zeros() as i32;
        (m << l, DENORM_UNIT_EXP + 63 - l)
    }
}

/// Round the exact value `±(m·2^e + ε)`, `0 ≤ ε < 2^e` known only by
/// `sticky` (ε ≠ 0), to the 80-bit format. When `sticky` is set the MSB
/// of `m` must be at bit ≥ 64 (the remainder then sits strictly below
/// the guard bit); below that, `m` is exact (cancellation), which
/// callers guarantee.
///
/// Handles normalization, round-to-nearest-even at 64 bits, the denormal
/// grid (rounding once, at the final precision), and overflow to
/// infinity.
fn pack(s: bool, m: u128, e: i32, sticky: bool) -> LDouble {
    if m == 0 {
        // Only exact zero reaches pack with m == 0 (a lone remainder
        // below 2^e always rounds away or to zero before this).
        return zero(s);
    }
    let msb = 127 - m.leading_zeros() as i32;
    let e2 = e + msb; // floor(log2 |value|)
    if e2 < E_MIN {
        return denormal_pack(s, m, e2, sticky);
    }
    // Normal result: significand M in [2^63, 2^64), guard bit, sticky.
    let mut mant: u64;
    let g: bool;
    let st: bool;
    let mut e2 = e2;
    if msb >= 64 {
        let sh = msb - 64;
        let mut m2 = m >> sh; // in [2^64, 2^65)
        st = sticky || (sh > 0 && m & ((1u128 << sh) - 1) != 0);
        g = m2 & 1 != 0;
        m2 >>= 1;
        mant = m2 as u64;
    } else if msb == 63 {
        mant = m as u64;
        g = false;
        st = sticky;
    } else {
        debug_assert!(!sticky);
        mant = (m << (63 - msb)) as u64;
        g = false;
        st = false;
    }
    if g && (st || mant & 1 != 0) {
        // Round up (a full significand carries into a new exponent).
        if mant == u64::MAX {
            mant = 1u64 << 63;
            e2 += 1;
        } else {
            mant += 1;
        }
    }
    if e2 > E_MAX {
        return infinity(s);
    }
    make(s, (e2 + BIAS) as u16, mant)
}

/// `pack` with `floor(log2 |value|) < E_MIN`: round onto the 2^-16445
/// grid in a single step (no double rounding).
fn denormal_pack(s: bool, m: u128, e2: i32, sticky: bool) -> LDouble {
    // Grid multiple = m·2^(e + 16445); shift = -(e + 16445) > 0.
    // With e = e2 - msb and the significand bit 0 at 2^(e2 - 63):
    // shift = 63 - e2 + 16445 - 63 = -(e2 + 16445) computed from raw m.
    let msb = 127 - m.leading_zeros() as i32;
    let e = e2 - msb;
    let t = -(e + 16445);
    debug_assert!(t > 0);
    if t > 128 {
        // Quotient < (2^128)·2^-129 < 0.5: below half the smallest
        // denormal, always zero.
        return zero(s);
    }
    if t == 128 {
        // Quotient = m·2^-128 ∈ [0.5, 1): the guard bit is bit 127.
        // Rounds up to the smallest denormal unless it is an exact
        // halfway (which goes to even, 0).
        let g = m >> 127 != 0;
        let st = sticky || m & ((1u128 << 127) - 1) != 0;
        return if g && st { make(s, 0, 1) } else { zero(s) };
    }
    let st = sticky || t > 1 && m & ((1u128 << (t - 1)) - 1) != 0;
    let g = t > 0 && m & (1u128 << (t - 1)) != 0;
    let mut v = (m >> t) as u64;
    if g && (st || v & 1 != 0) {
        v += 1;
    }
    if v >= 1u64 << 63 {
        // Rounded up to (or through) the smallest normal: encode the
        // pseudo-denormal field 2^63 as the normal 2^-16382.
        return make(s, 1, 1u64 << 63);
    }
    if v == 0 {
        return zero(s);
    }
    make(s, 0, v)
}

// ---------------------------------------------------------------------
// Conversions

/// `(long double) x` (`fld`): exact for every `f64`, NaN payloads shifted
/// into place, signaling NaNs left signaling.
pub fn from_f64(x: f64) -> LDouble {
    let b = x.to_bits();
    let s = b >> 63 != 0;
    let e = ((b >> 52) & 0x7ff) as i32;
    let frac = b & ((1u64 << 52) - 1);
    if e == 0x7ff {
        return if frac == 0 {
            infinity(s)
        } else {
            // NaN: shift the payload into place. `fld` quiets signaling
            // NaNs (bit 62 set), like every x87 load.
            make(s, 0x7fff, (1u64 << 63) | (frac << 11) | (1u64 << 62))
        };
    }
    if e == 0 {
        if frac == 0 {
            return zero(s);
        }
        // f64 denormal: a normal 80-bit value (2^-1074 > 2^-16382).
        pack(s, frac as u128, -1074, false)
    } else {
        make(s, (e - 1023 + BIAS) as u16, (1u64 << 63) | (frac << 11))
    }
}

/// `(long double) x` for an integer (`fild`): exact.
pub fn from_i64(x: i64) -> LDouble {
    if x == 0 {
        return zero(false);
    }
    let s = x < 0;
    let a = x.unsigned_abs();
    let l = a.leading_zeros() as i32;
    make(s, (63 - l + BIAS) as u16, a << l)
}

/// `(double) x` (`fstp qword`): round to nearest even; overflow to
/// infinity, underflow to f64 denormals; sNaNs quieted, payloads kept.
pub fn to_f64(x: &LDouble) -> f64 {
    let (c, s) = class(x);
    match c {
        Class::Zero => return if s { -0.0 } else { 0.0 },
        Class::Inf => return if s { f64::NEG_INFINITY } else { f64::INFINITY },
        Class::QNan | Class::SNan => {
            let q = if c == Class::SNan { split(x).1 | 0x4000_0000_0000_0000 } else { split(x).1 };
            return f64::from_bits((s as u64) << 63 | 0x7ff0_0000_0000_0000 | q >> 11);
        }
        // Unsupported encodings (pseudo-NaN, unnormal) load as the real
        // indefinite QNaN on x87; `fst qword` of that is the f64
        // indefinite (negative quiet NaN).
        Class::PseudoNan | Class::Unnormal => {
            return f64::from_bits(0xfff8_0000_0000_0000);
        }
        _ => {}
    }
    let (mant, e2) = norm(x);
    let sign = (s as u64) << 63;
    if e2 > 1023 {
        return f64::from_bits(sign | 0x7ff0_0000_0000_0000);
    }
    if e2 >= -1022 {
        // Normal: round the 64-bit significand to 53 bits.
        let mut f = mant >> 11;
        let g = (mant >> 10) & 1 != 0;
        let st = mant & ((1 << 10) - 1) != 0;
        if g && (st || f & 1 != 0) {
            f += 1;
        }
        if f == 1u64 << 53 {
            // Carried to 2^(e2+1): if that overflows, infinity.
            if e2 + 1 > 1023 {
                return f64::from_bits(sign | 0x7ff0_0000_0000_0000);
            }
            return f64::from_bits(sign | (((e2 + 1 + 1023) as u64) << 52));
        }
        return f64::from_bits(sign | (((e2 + 1023) as u64) << 52) | (f & ((1 << 52) - 1)));
    }
    // Subnormal f64: the grid is 2^-1074; value = mant·2^(e2-63).
    let sh = -(e2 - 63 + 1074); // = -(e2 + 1011) >= 12
    debug_assert!(sh >= 1);
    if sh > 64 {
        // Below half the smallest denormal (value < 2^-1075): ±0.
        return f64::from_bits(sign);
    }
    if sh == 64 {
        // value ∈ [2^-1075, 2^-1074): rounds to ±0 or ±(2^-1074); the
        // exact tie mant == 2^63 goes to even (0).
        let g = mant >> 63 != 0;
        let st = mant & 0x7fff_ffff_ffff_ffff != 0;
        return f64::from_bits(sign | (g && st) as u64);
    }
    let g = (mant >> (sh - 1)) & 1 != 0;
    let st = sh > 1 && mant & ((1u64 << (sh - 1)) - 1) != 0;
    let mut v = mant >> sh;
    if g && (st || v & 1 != 0) {
        v += 1;
    }
    f64::from_bits(sign | v)
}

// ---------------------------------------------------------------------
// Arithmetic

/// `faddp`/`fsubp` with destination `a` and source `b`: `a ± b`.
fn addsub(a: &LDouble, b: &LDouble, sub: bool) -> LDouble {
    let (ca, sa) = class(a);
    let (cb, sb0) = class(b);
    let sb = sb0 ^ sub;
    if is_nan(ca) || is_nan(cb) {
        return pick_nan(a, b);
    }
    if ca == Class::Inf {
        if cb == Class::Inf {
            return if sa == sb { *a } else { indefinite() };
        }
        return *a;
    }
    if cb == Class::Inf {
        return infinity(sb);
    }
    if ca == Class::Zero && cb == Class::Zero {
        // Sum of zeros: -0 only when both "effective" signs are negative.
        return zero(sa && sb);
    }
    if ca == Class::Zero {
        return with_sign(&canon_enc(b), sb);
    }
    if cb == Class::Zero {
        return with_sign(&canon_enc(a), sa);
    }
    let (ma, ea) = norm(a);
    let (mb, eb) = norm(b);
    if (ea, ma) < (eb, mb) { add_core(mb, eb, ma, ea, sb, sa != sb) } else { add_core(ma, ea, mb, eb, sa, sa != sb) }
}

/// Add/subtract two normalized nonzero significands; `(ma, ea)` is the
/// larger magnitude, `s` the result sign, `sub` whether the smaller is
/// subtracted.
fn add_core(ma: u64, ea: i32, mb: u64, eb: i32, s: bool, sub: bool) -> LDouble {
    let de = ea - eb;
    debug_assert!(de >= 0);
    if de > 64 {
        // Subtracting from a power of two can land in the finer binade
        // below: with de == 65 the difference is a strict halfway or
        // beyond, so it rounds to (2^64-1)·2^(ea-64) — except for an
        // exact tie (mb == 2^63), which rounds back to a. Otherwise the
        // smaller operand is below half an ulp: the exact sum rounds to
        // the larger operand either way.
        if de == 65 && ma == 1u64 << 63 && sub && ea >= E_MIN && mb > 1u64 << 63 {
            return if ea - 1 >= E_MIN { make(s, (ea - 1 + BIAS) as u16, u64::MAX) } else { make(s, 0, u64::MAX) };
        }
        return make(s, (ea + BIAS) as u16, ma);
    }
    // Exact window: for de <= 63 the larger operand's bit 63 sits at
    // bit 126 and the sum's carry still fits; for de == 64 the smaller
    // mantissa needs no shift and ma·2^64 ± mb < 2^128. Rounding
    // happens once, in `pack`.
    let (r, e) = if de == 64 {
        let big = (ma as u128) << 64;
        if sub { (big - mb as u128, ea - 127) } else { (big + mb as u128, ea - 127) }
    } else {
        let big = (ma as u128) << 63;
        let small = (mb as u128) << (63 - de);
        if sub { (big - small, ea - 126) } else { (big + small, ea - 126) }
    };
    if r == 0 {
        return zero(false); // exact cancellation: +0 in round-to-nearest
    }
    pack(s, r, e, false)
}

pub fn add(a: &LDouble, b: &LDouble) -> LDouble {
    addsub(a, b, false)
}

pub fn sub(a: &LDouble, b: &LDouble) -> LDouble {
    addsub(a, b, true)
}

pub fn mul(a: &LDouble, b: &LDouble) -> LDouble {
    let (ca, sa) = class(a);
    let (cb, sb) = class(b);
    if is_nan(ca) || is_nan(cb) {
        return pick_nan(a, b);
    }
    let s = sa ^ sb;
    if ca == Class::Inf || cb == Class::Inf {
        if ca == Class::Zero || cb == Class::Zero {
            return indefinite(); // 0 * Inf
        }
        return infinity(s);
    }
    if ca == Class::Zero || cb == Class::Zero {
        return zero(s);
    }
    let (ma, ea) = norm(a);
    let (mb, eb) = norm(b);
    let p = (ma as u128) * (mb as u128);
    pack(s, p, ea + eb - 126, false)
}

pub fn div(a: &LDouble, b: &LDouble) -> LDouble {
    let (ca, sa) = class(a);
    let (cb, sb) = class(b);
    if is_nan(ca) || is_nan(cb) {
        return pick_nan(a, b);
    }
    let s = sa ^ sb;
    if ca == Class::Inf {
        if cb == Class::Inf {
            return indefinite();
        }
        return infinity(s);
    }
    if cb == Class::Inf {
        return zero(s);
    }
    if cb == Class::Zero {
        if ca == Class::Zero {
            return indefinite(); // 0 / 0
        }
        return infinity(s); // x / 0 (divide-by-zero, value ±Inf)
    }
    if ca == Class::Zero {
        return zero(s);
    }
    let (ma, ea) = norm(a);
    let (mb, eb) = norm(b);
    // q = (ma << 64) / mb ∈ [2^63, 2^65): quotient with guard bit, the
    // remainder carries the sticky information.
    let num = (ma as u128) << 64;
    let q = num / (mb as u128);
    let r = num - q * (mb as u128);
    if q < (1u128 << 64) {
        // Bring a guard bit out of the remainder: 2r >= mb.
        let r2 = r << 1;
        let bit = r2 >= (mb as u128);
        let r3 = if bit { r2 - (mb as u128) } else { r2 };
        pack(s, (q << 1) | (bit as u128), ea - eb - 65, r3 != 0)
    } else {
        pack(s, q, ea - eb - 64, r != 0)
    }
}

/// `fsqrt`: correctly rounded, as on the FPU.
pub fn sqrt(x: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Inf => {
            return if s { indefinite() } else { *x };
        }
        Class::Zero => return zero(s), // sqrt(-0) = -0
        _ => {}
    }
    if s {
        return indefinite(); // sqrt(negative)
    }
    let (m, e2) = norm(x);
    // value = m·2^p with p = e2-63; sqrt = s·2^e_s with s ∈ [2^63, 2^64):
    // make p even, then s = isqrt(m' << 64 or << 62) (whichever fits
    // normalized) and e_s = 63 + (p' - shift)/2.
    let p = e2 - 63;
    let (n, e_s) = if p & 1 == 0 {
        ((m as u128) << 64, (p - 64) / 2 + 63)
    } else {
        // m·2^p = (2m)·2^(p-1), and (2m)·2^62 ∈ [2^126, 2^127).
        (((m as u128) << 1) << 62, (p - 1 - 62) / 2 + 63)
    };
    let (sq, rem) = isqrt128(n);
    // True value = sqrt(sq² + rem)·2^(e_s - 63); the fraction f of
    // sqrt(sq²+rem) satisfies f > 1/2 iff rem > sq + 1/4, and f == 1/2
    // is impossible (no tie).
    let mut mant = sq;
    let mut e2 = e_s;
    if rem >= (sq as u128) + 1 {
        if sq == u64::MAX {
            // Carried past a full significand.
            e2 += 1;
            if e2 > E_MAX {
                return infinity(false);
            }
            return make(false, (e2 + BIAS) as u16, 1u64 << 63);
        }
        mant += 1;
    }
    make(false, (e2 + BIAS) as u16, mant)
}

/// `floor(sqrt(n))` and the remainder, n < 2^128 (all comparisons via
/// division: `s² ≤ n ⟺ s ≤ n/s`, no overflow).
fn isqrt128(n: u128) -> (u64, u128) {
    // Newton from an f64 seed (within a few units of the root).
    let seed = if n >> 64 == 0 {
        (n as u64 as f64).sqrt() as u128
    } else {
        (((n >> 64) as f64).sqrt() * 4294967296.0f64) as u128
    };
    let mut s = seed.max(1);
    loop {
        let t = (s + n / s) >> 1;
        if t >= s {
            break;
        }
        s = t;
    }
    while s > 1 && s > n / s {
        s -= 1;
    }
    let mut s1 = s + 1;
    while s1 <= n / s1 {
        s = s1;
        s1 += 1;
    }
    (s as u64, n - s * s)
}

/// `frndint` in the default rounding mode (nearest even). Values with
/// |x| >= 2^63 are left unchanged, as the FPU does; denormals round to
/// ±0 (the largest is below 1).
pub fn rint(x: &LDouble) -> LDouble {
    let (c, s) = class(x);
    match c {
        Class::QNan => return *x,
        Class::SNan => return quiet(x),
        Class::PseudoNan | Class::Unnormal => return indefinite(),
        Class::Inf | Class::Zero => return *x,
        _ => {}
    }
    let (m, e2) = norm(x);
    if e2 >= 63 {
        return *x;
    }
    if e2 < -1 {
        return zero(s);
    }
    let int: u64 = if e2 == -1 {
        // value in [0.5, 1): rounds to 1 only above 0.5 (tie goes to 0).
        (m > 1u64 << 63) as u64
    } else {
        let sh = (63 - e2) as u32; // 0..=63
        if sh == 0 {
            m
        } else {
            let frac = m & ((1u64 << sh) - 1);
            let mut i = m >> sh;
            let half = 1u64 << (sh - 1);
            if frac > half || frac == half && i & 1 != 0 {
                i += 1;
            }
            i
        }
    };
    if int == 0 {
        return zero(s);
    }
    let l = int.leading_zeros() as i32;
    make(s, (63 - l + BIAS) as u16, int << l)
}

/// `fucompp`-style three-way compare: -1, 0, 1, or 2 for unordered.
pub fn cmp(a: &LDouble, b: &LDouble) -> i32 {
    let (ca, sa) = class(a);
    let (cb, sb) = class(b);
    if is_nan(ca) || is_nan(cb) {
        return 2;
    }
    if ca == Class::Zero && cb == Class::Zero {
        return 0; // +0 == -0
    }
    if ca == Class::Zero {
        return if sb { 1 } else { -1 };
    }
    if cb == Class::Zero {
        return if sa { -1 } else { 1 };
    }
    if ca == Class::Inf || cb == Class::Inf {
        let ord = if ca == Class::Inf && cb == Class::Inf {
            return if sa == sb {
                0
            } else if sa {
                -1
            } else {
                1
            };
        } else if ca == Class::Inf {
            if sa { -1 } else { 1 }
        } else {
            if sb { 1 } else { -1 }
        };
        return ord;
    }
    let (ma, ea) = norm(a);
    let (mb, eb) = norm(b);
    use std::cmp::Ordering::*;
    if sa != sb {
        return if sa { -1 } else { 1 };
    }
    let mag = if ea != eb { ea.cmp(&eb) } else { ma.cmp(&mb) };
    let dir = if sa { mag.reverse() } else { mag };
    match dir {
        Less => -1,
        Equal => 0,
        Greater => 1,
    }
}

// ---------------------------------------------------------------------
// Loop kernels: sequential operations, each exactly rounded like the
// x87 loops (so the software path reproduces them bit for bit).

pub fn sum_from(init: &LDouble, xs: &[f64]) -> LDouble {
    let mut s = *init;
    for &x in xs {
        s = add(&s, &from_f64(x));
    }
    s
}

pub fn prod_from(init: &LDouble, xs: &[f64]) -> LDouble {
    let mut p = *init;
    for &x in xs {
        p = mul(&p, &from_f64(x));
    }
    p
}

pub fn cumsum_from(init: &LDouble, xs: &[f64], ys: &mut [f64]) -> LDouble {
    let mut s = *init;
    for (&x, y) in xs.iter().zip(ys.iter_mut()) {
        s = add(&s, &from_f64(x));
        *y = to_f64(&s);
    }
    s
}

pub fn cumprod_from(init: &LDouble, xs: &[f64], ys: &mut [f64]) -> LDouble {
    let mut p = *init;
    for (&x, y) in xs.iter().zip(ys.iter_mut()) {
        p = mul(&p, &from_f64(x));
        *y = to_f64(&p);
    }
    p
}

pub fn sum_dev(m: &LDouble, xs: &[f64]) -> LDouble {
    let mut t = zero(false);
    for &x in xs {
        t = add(&t, &sub(&from_f64(x), m));
    }
    t
}

pub fn sum_sq_dev(m: &LDouble, xs: &[f64]) -> LDouble {
    let mut t = zero(false);
    for &x in xs {
        let d = sub(&from_f64(x), m);
        t = add(&t, &mul(&d, &d));
    }
    t
}

pub fn sum_cross_dev(xm: &LDouble, xs: &[f64], ym: &LDouble, ys: &[f64]) -> LDouble {
    let mut t = zero(false);
    for (&x, &y) in xs.iter().zip(ys) {
        let dx = sub(&from_f64(x), xm);
        let dy = sub(&from_f64(y), ym);
        t = add(&t, &mul(&dx, &dy));
    }
    t
}

pub fn add_each(acc: &mut [LDouble], xs: &[f64]) {
    for (a, &x) in acc.iter_mut().zip(xs) {
        *a = add(a, &from_f64(x));
    }
}
