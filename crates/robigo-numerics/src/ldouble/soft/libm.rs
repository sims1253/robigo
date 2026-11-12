//! `expl`, `logl` and `log1pl` as glibc 2.41 implements them on x86_64,
//! ported instruction by instruction, so that targets without glibc's
//! x87 libm (aarch64, macOS) give x86_64 GNU R's long-double results.
//!
//! - R: sysdeps/x86_64/fpu/e_expl.S:__ieee754_expl
//! - R: sysdeps/x86_64/fpu/e_logl.S:__ieee754_logl, behind the SVID
//!   wrapper math/w_logl_compat.c:__logl that the `logl` symbol is
//!   (the `expl` wrapper only re-derives the values the kernel returns,
//!   and `log1pl` has none that changes values)
//! - R: sysdeps/x86_64/fpu/s_log1pl.S:__log1pl
//!
//! Every x87 arithmetic instruction is the matching (exactly rounded)
//! soft operation. The microcode instructions `f2xm1`, `fyl2x` and
//! `fyl2xp1` come from a [`Microcode`]: correctly rounded ([`Exact`]) in
//! the library, the hardware's in the property tests (see
//! `microcode.rs` for why the two differ in the last bit).

use super::microcode::{Exact, Microcode};
use super::pow::{M_LOG2E, fscale, ld, pow2};
use super::{Class, LDouble, add, class, cmp, infinity, make, mul, rint, split, sub, with_sign, zero};

/// `fldln2`: ln 2 rounded to 64 bits.
const LN2: LDouble = ld(0xb172_17f7_d1cf_79ac, -64);
/// e_expl.S `c0`: the top 16 bits of log2(e).
const C0: LDouble = ld(0xb8aa_0000_0000_0000, -63);
/// e_expl.S `c1`: log2(e) − c0.
const C1: LDouble = ld(0xeca5_705f_c2ee_fa20, -81);
/// e_expl.S `csat`: 2^15, past which expl overflows or underflows.
const CSAT: LDouble = pow2(15);
/// e_logl.S `limit`: the double 0.29.
const LIMIT_D: LDouble = ld(0x947a_e147_ae14_7800, -65);
/// s_log1pl.S `limit`: 0.29 as a long double.
const LIMIT_L: LDouble = ld(0x947a_e147_ae14_7ae1, -65);

fn fabs(x: &LDouble) -> LDouble {
    with_sign(x, false)
}

/// R: e_expl.S:__ieee754_expl.
pub fn expl(x: &LDouble) -> LDouble {
    expl_with::<Exact>(x)
}

/// [`expl`] on the microcode `M`.
pub(crate) fn expl_with<M: Microcode>(x: &LDouble) -> LDouble {
    let (se, _) = split(x);
    let mut x = *x;
    let ex = se & 0x7fff;
    if ex > 0x400d {
        let (c, s) = class(&x);
        match c {
            Class::Inf => return if s { zero(false) } else { x },
            Class::QNan | Class::SNan => return add(&x, &x),
            // `fxam` calls the rest finite: saturate, keeping the sign.
            _ => x = with_sign(&CSAT, s),
        }
    } else if ex < 0x3fbc {
        // |x| < 2^-67: the result rounds to 1.
        return add(&LDouble::ONE, &x);
    }
    let i = rint(&mul(&M_LOG2E, &x));
    let xi = rint(&x);
    let f = sub(&mul(&C0, &xi), &i);
    let xf = sub(&x, &xi);
    let f = add(&f, &mul(&C0, &xf));
    let f = add(&f, &mul(&C1, &x));
    let r = add(&M::f2xm1(&f), &LDouble::ONE);
    fscale(&r, &i)
}

/// R: w_logl_compat.c:__logl and e_logl.S:__ieee754_logl.
pub fn logl(x: &LDouble) -> LDouble {
    logl_with::<Exact>(x)
}

/// [`logl`] on the microcode `M`.
pub(crate) fn logl_with<M: Microcode>(x: &LDouble) -> LDouble {
    // The wrapper: x ≤ 0 returns __kernel_standard_l's -HUGE_VAL for a
    // zero and its (double) NAN, the positive default NaN, otherwise.
    match cmp(x, &zero(false)) {
        0 => return infinity(true),
        -1 => return make(false, 0x7fff, 0xc000_0000_0000_0000),
        _ => {}
    }
    let (c, _) = class(x);
    if matches!(c, Class::QNan | Class::SNan) {
        return add(x, x);
    }
    let (se, _) = split(x);
    // x ≤ −2 skips the subtraction (it could overflow); fyl2x then
    // sees a negative operand either way.
    let t = if se >= 0xc000 { *x } else { sub(x, &LDouble::ONE) };
    if cmp(&fabs(&t), &LIMIT_D) == 1 {
        return M::fyl2x(x, &LN2);
    }
    let t = if class(&t).0 == Class::Zero { fabs(&t) } else { t }; // log(1) = +0
    M::fyl2xp1(&t, &LN2)
}

/// R: s_log1pl.S:__log1pl.
pub fn log1pl(x: &LDouble) -> LDouble {
    log1pl_with::<Exact>(x)
}

/// [`log1pl`] on the microcode `M`.
pub(crate) fn log1pl_with<M: Microcode>(x: &LDouble) -> LDouble {
    let (c, _) = class(x);
    if matches!(c, Class::QNan | Class::SNan) {
        return add(x, x);
    }
    // `fcompp` of the limit against |x|: C0 (limit < |x|, or unordered)
    // takes fyl2x.
    if matches!(cmp(&LIMIT_L, &fabs(x)), 0 | 1) {
        return M::fyl2xp1(x, &LN2);
    }
    let (se, _) = split(x);
    if se ^ 0x8000 >= 0xc040 {
        // x ≥ 2^65: x + 1 would round to x.
        return M::fyl2x(x, &LN2);
    }
    M::fyl2x(&add(x, &LDouble::ONE), &LN2)
}

#[cfg(test)]
mod tests {
    use super::super::from_f64;
    use super::*;

    fn bits(x: &LDouble) -> [u8; 10] {
        x.0[..10].try_into().unwrap()
    }

    #[test]
    fn constants() {
        assert_eq!(bits(&LIMIT_D), bits(&from_f64(0.29)));
        assert_eq!(bits(&CSAT), bits(&from_f64(32768.0)));
    }

    #[test]
    fn glibc_bits() {
        // The 80-bit words glibc's x87 libm gives (see ldmath.rs).
        let words =
            |x: LDouble| -> [u16; 5] { std::array::from_fn(|k| u16::from_le_bytes([x.0[2 * k], x.0[2 * k + 1]])) };
        assert_eq!(words(expl(&from_f64(1.0))), [0x4a9b, 0xa2bb, 0x5458, 0xadf8, 0x4000]);
        assert_eq!(words(logl(&from_f64(2.0))), [0x79ac, 0xd1cf, 0x17f7, 0xb172, 0x3ffe]);
    }

    #[test]
    fn special_values() {
        let f = |x: LDouble| super::super::to_f64(&x);
        assert_eq!(f(expl(&from_f64(0.0))), 1.0);
        assert_eq!(f(expl(&from_f64(f64::NEG_INFINITY))), 0.0);
        assert_eq!(f(expl(&from_f64(f64::INFINITY))), f64::INFINITY);
        assert_eq!(f(expl(&from_f64(1e6))), f64::INFINITY);
        assert_eq!(f(expl(&from_f64(-1e6))), 0.0);
        assert!(f(expl(&from_f64(f64::NAN))).is_nan());
        assert_eq!(f(logl(&from_f64(1.0))).to_bits(), 0.0f64.to_bits());
        assert_eq!(f(logl(&from_f64(0.0))), f64::NEG_INFINITY);
        assert_eq!(f(logl(&from_f64(-0.0))), f64::NEG_INFINITY);
        assert_eq!(f(logl(&from_f64(f64::INFINITY))), f64::INFINITY);
        assert!(f(logl(&from_f64(-1.0))).is_nan());
        assert!(f(logl(&from_f64(f64::NEG_INFINITY))).is_nan());
        assert_eq!(f(log1pl(&from_f64(-0.0))).to_bits(), (-0.0f64).to_bits());
        assert_eq!(f(log1pl(&from_f64(-1.0))), f64::NEG_INFINITY);
        assert_eq!(f(log1pl(&from_f64(f64::INFINITY))), f64::INFINITY);
        assert!(f(log1pl(&from_f64(-2.0))).is_nan());
        assert_eq!(f(log1pl(&from_f64(1e300))), 1e300f64.ln());
    }
}
