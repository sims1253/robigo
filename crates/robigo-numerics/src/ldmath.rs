//! The C library's `long double` math functions that nmath uses on
//! `LDOUBLE` values (`expl`, `logl`, `log1pl`), with GNU R's semantics:
//! glibc's implementations, the ones the oracle's GNU R links against.
//!
//! On x86_64 glibc the values go to and from the C library in the x87
//! format, so the results are bit-identical to GNU R's. Elsewhere (no
//! x87, or another C library whose results differ in the last bits) they
//! come from the software port of glibc's code in `ldouble::soft`.

use crate::ldouble::LDouble;

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
mod imp {
    use super::LDouble;
    use std::arch::asm;

    unsafe extern "C" {
        #[link_name = "expl"]
        fn c_expl();
        #[link_name = "logl"]
        fn c_logl();
        #[link_name = "log1pl"]
        fn c_log1pl();
    }

    macro_rules! unary {
        ($name:ident, $c:ident) => {
            /// SysV: a `long double` argument is passed in memory (16 bytes),
            /// the result returned in st(0). The output pointer is in a
            /// callee-saved register so that it survives the call.
            pub fn $name(x: &LDouble) -> LDouble {
                let mut out = LDouble([0; 16]);
                let xp = x as *const LDouble as *const u8;
                let op = out.0.as_mut_ptr();
                // SAFETY: the C function of type `long double (long double)`,
                // called with the stack aligned for it (no `nostack`), its
                // argument in place and its result popped into `out`.
                unsafe {
                    asm!(
                        "sub rsp, 32",
                        "fld tbyte ptr [{x}]",
                        "fstp tbyte ptr [rsp]",
                        "call {f}",
                        "add rsp, 32",
                        "fstp tbyte ptr [r12]",
                        x = in(reg) xp,
                        in("r12") op,
                        f = sym $c,
                        clobber_abi("C"),
                    );
                }
                out
            }
        };
    }

    unary!(expl, c_expl);
    unary!(logl, c_logl);
    unary!(log1pl, c_log1pl);
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")))]
mod imp {
    pub use crate::ldouble::soft::{expl, log1pl, logl};
}

/// Compare two long doubles: -1, 0, 1, or 2 for unordered.
#[inline]
pub fn cmp(a: &LDouble, b: &LDouble) -> i32 {
    a.cmp_code(b)
}

/// `expl(x)`.
#[inline]
pub fn expl(x: LDouble) -> LDouble {
    imp::expl(&x)
}

/// `logl(x)`.
#[inline]
pub fn logl(x: LDouble) -> LDouble {
    imp::logl(&x)
}

/// `log1pl(x)`.
#[inline]
pub fn log1pl(x: LDouble) -> LDouble {
    imp::log1pl(&x)
}

/// `fmax2` on long doubles.
#[inline]
pub fn fmax2l(x: LDouble, y: LDouble) -> LDouble {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if cmp(&x, &y) < 0 { y } else { x }
}

/// `fmin2` on long doubles.
#[inline]
pub fn fmin2l(x: LDouble, y: LDouble) -> LDouble {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if cmp(&x, &y) < 0 { x } else { y }
}

/// `(x > y)` for long doubles (false when either is NaN).
#[inline]
pub fn gt(x: LDouble, y: LDouble) -> bool {
    cmp(&x, &y) == 1
}

/// `(x >= y)` for long doubles.
#[inline]
pub fn ge(x: LDouble, y: LDouble) -> bool {
    matches!(cmp(&x, &y), 0 | 1)
}

/// `(x < y)` for long doubles.
#[inline]
pub fn lt(x: LDouble, y: LDouble) -> bool {
    cmp(&x, &y) == -1
}

/// `(x <= y)` for long doubles.
#[inline]
pub fn le(x: LDouble, y: LDouble) -> bool {
    matches!(cmp(&x, &y), 0 | -1)
}

/// `(x == y)` for long doubles.
#[inline]
pub fn eq(x: LDouble, y: LDouble) -> bool {
    cmp(&x, &y) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ld(x: f64) -> LDouble {
        LDouble::from_f64(x)
    }

    #[test]
    fn logl_expl() {
        assert_eq!(logl(ld(2.0)).to_f64(), 2.0f64.ln());
        assert_eq!(expl(ld(1.0)).to_f64(), std::f64::consts::E);
        assert_eq!(expl(logl(ld(10.0))).to_f64(), 10.0);
        assert_eq!(log1pl(ld(1e-30)).to_f64(), 1e-30f64.ln_1p());
        assert_eq!(log1pl(ld(-0.5)).to_f64(), 0.5f64.ln());
        assert_eq!(expl(ld(0.0)).to_f64(), 1.0);
        assert_eq!(logl(ld(1.0)).to_f64(), 0.0);
    }

    #[test]
    fn comparisons() {
        assert!(gt(ld(2.0), ld(1.0)));
        assert!(!gt(ld(1.0), ld(1.0)));
        assert!(ge(ld(1.0), ld(1.0)));
        assert!(lt(ld(1.0), ld(2.0)));
        assert!(le(ld(1.0), ld(1.0)));
        assert!(eq(ld(1.0), ld(1.0)));
        assert!(!gt(ld(f64::NAN), ld(1.0)));
        assert!(!lt(ld(f64::NAN), ld(1.0)));
        assert_eq!(fmax2l(ld(1.0), ld(2.0)).to_f64(), 2.0);
        assert_eq!(fmin2l(ld(1.0), ld(2.0)).to_f64(), 1.0);
    }

    #[test]
    fn exact_glibc_bits() {
        // The 80-bit results glibc gives for these inputs (printed by C as
        // the raw significand/exponent words).
        fn bits(x: LDouble) -> [u16; 5] {
            std::array::from_fn(|k| u16::from_le_bytes([x.0[2 * k], x.0[2 * k + 1]]))
        }
        // expl(1.0L) = 2.71828182845904523536028747135266249776L
        assert_eq!(bits(expl(ld(1.0))), [0x4a9b, 0xa2bb, 0x5458, 0xadf8, 0x4000]);
        // logl(2.0L) = 0.693147180559945309417232121458176568076L
        assert_eq!(bits(logl(ld(2.0))), [0x79ac, 0xd1cf, 0x17f7, 0xb172, 0x3ffe]);
    }
}
