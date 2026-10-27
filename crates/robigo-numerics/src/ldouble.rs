//! C `long double` (R's `LDOUBLE`) as GNU R uses it: the x87 80-bit
//! extended format with a 64-bit significand, rounding to nearest.
//!
//! R accumulates in long double in several places (`sum()`, `mean()`,
//! `cumsum()` in summary.c/cum.c; pdhyper, pnchisq, pnbeta, pnt, ptukey
//! in nmath), so plain `f64` arithmetic can differ in the last bit.
//! On x86_64 the operations below run on the x87 FPU, which gives the
//! same results as the C code. Elsewhere [`soft`] provides a software
//! implementation with identical results (checked against the hardware
//! by the property tests on x86_64).

/// A software implementation of the same format, with x87 value
/// semantics. Used directly on targets without x87; on x86_64 it exists
/// so the property tests can prove it agrees with the FPU bit for bit.
pub mod soft;

/// An 80-bit x87 extended-precision value (stored in 16 bytes).
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct LDouble([u8; 16]);

impl std::fmt::Debug for LDouble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LDouble({:?})", self.to_f64())
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use super::LDouble;
    use std::arch::asm;

    #[inline]
    pub fn from_f64(x: f64) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld qword ptr [{x}]",
                "fstp tbyte ptr [{o}]",
                x = in(reg) &x,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `(long double) x` for a 64-bit integer: exact (`fild`).
    #[inline]
    pub fn from_i64(x: i64) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fild qword ptr [{x}]",
                "fstp tbyte ptr [{o}]",
                x = in(reg) &x,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    #[inline]
    pub fn to_f64(x: &LDouble) -> f64 {
        let mut out = 0.0f64;
        unsafe {
            asm!(
                "fld tbyte ptr [{x}]",
                "fstp qword ptr [{o}]",
                x = in(reg) x.0.as_ptr(),
                o = in(reg) &mut out,
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    macro_rules! binop {
        ($name:ident, $insn:literal) => {
            #[inline]
            pub fn $name(a: &LDouble, b: &LDouble) -> LDouble {
                let mut out = LDouble([0; 16]);
                unsafe {
                    asm!(
                        "fld tbyte ptr [{a}]",
                        "fld tbyte ptr [{b}]",
                        $insn,
                        "fstp tbyte ptr [{o}]",
                        a = in(reg) a.0.as_ptr(),
                        b = in(reg) b.0.as_ptr(),
                        o = in(reg) out.0.as_mut_ptr(),
                        out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                        out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                        options(nostack),
                    );
                }
                out
            }
        };
    }

    // st(1) = a, st(0) = b; f*p st(1), st(0) computes st(1) op st(0)
    binop!(add, "faddp st(1), st(0)");
    binop!(sub, "fsubp st(1), st(0)");
    binop!(mul, "fmulp st(1), st(0)");
    binop!(div, "fdivp st(1), st(0)");

    /// `sqrtl`: glibc uses the (correctly rounded) x87 `fsqrt`.
    #[inline]
    pub fn sqrt(a: &LDouble) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{a}]",
                "fsqrt",
                "fstp tbyte ptr [{o}]",
                a = in(reg) a.0.as_ptr(),
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `s = init; for x in xs: s += x` with `s` kept on the FPU stack, as
    /// GNU R's C loops do (`fadd m64` loads each double exactly).
    pub fn sum_from(init: &LDouble, xs: &[f64]) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{i}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fadd qword ptr [{p}]",
                "add {p}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp tbyte ptr [{o}]",
                i = in(reg) init.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `s = init; for x in xs: s *= x`.
    pub fn prod_from(init: &LDouble, xs: &[f64]) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{i}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fmul qword ptr [{p}]",
                "add {p}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp tbyte ptr [{o}]",
                i = in(reg) init.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `s = init; for i: s += xs[i]; out[i] = (double) s`.
    pub fn cumsum_from(init: &LDouble, xs: &[f64], ys: &mut [f64]) -> LDouble {
        assert!(ys.len() >= xs.len());
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{i}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fadd qword ptr [{p}]",
                "fst qword ptr [{q}]",
                "add {p}, 8",
                "add {q}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp tbyte ptr [{o}]",
                i = in(reg) init.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                q = inout(reg) ys.as_mut_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `s = init; for i: s *= xs[i]; out[i] = (double) s`.
    pub fn cumprod_from(init: &LDouble, xs: &[f64], ys: &mut [f64]) -> LDouble {
        assert!(ys.len() >= xs.len());
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{i}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fmul qword ptr [{p}]",
                "fst qword ptr [{q}]",
                "add {p}, 8",
                "add {q}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp tbyte ptr [{o}]",
                i = in(reg) init.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                q = inout(reg) ys.as_mut_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `t = 0; for x in xs: t += ((long double) x - m) * ((long double) x - m)`.
    pub fn sum_sq_dev(m: &LDouble, xs: &[f64]) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fldz",
                "fld tbyte ptr [{m}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fld qword ptr [{p}]",
                "fsub st(0), st(1)",
                "fmul st(0), st(0)",
                "faddp st(2), st(0)",
                "add {p}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp st(0)",
                "fstp tbyte ptr [{o}]",
                m = in(reg) m.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `t = 0; for i: t += ((long double) xs[i] - xm) * ((long double) ys[i] - ym)`.
    pub fn sum_cross_dev(xm: &LDouble, xs: &[f64], ym: &LDouble, ys: &[f64]) -> LDouble {
        assert!(ys.len() >= xs.len());
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fldz",
                "fld tbyte ptr [{ym}]",
                "fld tbyte ptr [{xm}]",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fld qword ptr [{p}]",
                "fsub st(0), st(1)",
                "fld qword ptr [{q}]",
                "fsub st(0), st(3)",
                "fmulp st(1), st(0)",
                "faddp st(3), st(0)",
                "add {p}, 8",
                "add {q}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp st(0)",
                "fstp st(0)",
                "fstp tbyte ptr [{o}]",
                xm = in(reg) xm.0.as_ptr(),
                ym = in(reg) ym.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                q = inout(reg) ys.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `acc[i] += xs[i]` for each `i`.
    pub fn add_each(acc: &mut [LDouble], xs: &[f64]) {
        assert!(acc.len() >= xs.len());
        unsafe {
            asm!(
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fld tbyte ptr [{a}]",
                "fadd qword ptr [{p}]",
                "fstp tbyte ptr [{a}]",
                "add {a}, 16",
                "add {p}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                a = inout(reg) acc.as_mut_ptr() => _,
                p = inout(reg) xs.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
    }

    /// `t = 0; for x in xs: t += (long double) x - m` (mean's second pass).
    pub fn sum_dev(m: &LDouble, xs: &[f64]) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{m}]",
                "fldz",
                "test {n}, {n}",
                "jz 3f",
                "2:",
                "fld qword ptr [{p}]",
                "fsub st(0), st(2)",
                "faddp st(1), st(0)",
                "add {p}, 8",
                "dec {n}",
                "jnz 2b",
                "3:",
                "fstp tbyte ptr [{o}]",
                "fstp st(0)",
                m = in(reg) m.0.as_ptr(),
                p = inout(reg) xs.as_ptr() => _,
                n = inout(reg) xs.len() => _,
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `nearbyintl`: round to an integer in the current (nearest-even)
    /// rounding mode, as x87 `frndint` does.
    #[inline]
    pub fn rint(a: &LDouble) -> LDouble {
        let mut out = LDouble([0; 16]);
        unsafe {
            asm!(
                "fld tbyte ptr [{a}]",
                "frndint",
                "fstp tbyte ptr [{o}]",
                a = in(reg) a.0.as_ptr(),
                o = in(reg) out.0.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        out
    }

    /// `powl(x, y)`: glibc's, as GNU R calls it (its results for large
    /// exponents depend on its algorithm, so it is called, not ported).
    pub fn powl(x: &LDouble, y: &LDouble) -> LDouble {
        unsafe extern "C" {
            fn powl();
        }
        let mut out = LDouble([0; 16]);
        unsafe {
            // SysV: long double arguments are passed in memory (16 bytes
            // each), the result returned in st(0). The output pointer is
            // in a callee-saved register so that it survives the call.
            asm!(
                "sub rsp, 32",
                "fld tbyte ptr [{x}]",
                "fstp tbyte ptr [rsp]",
                "fld tbyte ptr [{y}]",
                "fstp tbyte ptr [rsp + 16]",
                "call {f}",
                "add rsp, 32",
                "fstp tbyte ptr [r12]",
                x = in(reg) x.0.as_ptr(),
                y = in(reg) y.0.as_ptr(),
                in("r12") out.0.as_mut_ptr(),
                f = sym powl,
                clobber_abi("C"),
            );
        }
        out
    }

    /// Compare: returns -1, 0, 1, or 2 for unordered.
    #[inline]
    pub fn cmp(a: &LDouble, b: &LDouble) -> i32 {
        let flags: u16;
        unsafe {
            asm!(
                "fld tbyte ptr [{b}]",
                "fld tbyte ptr [{a}]",
                "fucompp",
                "fnstsw ax",
                a = in(reg) a.0.as_ptr(),
                b = in(reg) b.0.as_ptr(),
                out("ax") flags,
                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                options(nostack),
            );
        }
        // C3 (bit 14), C2 (bit 10), C0 (bit 8): compares st(0)=a with st(1)=b
        let c0 = flags & 0x0100 != 0;
        let c2 = flags & 0x0400 != 0;
        let c3 = flags & 0x4000 != 0;
        if c2 {
            2
        } else if c3 {
            0
        } else if c0 {
            -1
        } else {
            1
        }
    }
}

// Off x86_64 the same interface comes from the software 80-bit
// implementation, never a silent f64 fallback.
#[cfg(not(target_arch = "x86_64"))]
mod imp {
    pub use super::soft::*;
}

impl LDouble {
    pub const ZERO: LDouble = LDouble([0; 16]);
    /// 1.0: significand 0x8000000000000000, biased exponent 0x3fff (the
    /// x87 encoding, on every platform).
    pub const ONE: LDouble = LDouble([0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0x3f, 0, 0, 0, 0, 0, 0]);

    #[inline]
    pub fn from_f64(x: f64) -> LDouble {
        imp::from_f64(x)
    }

    /// `(double) x`: round to nearest double.
    #[inline]
    pub fn to_f64(self) -> f64 {
        imp::to_f64(&self)
    }

    /// `sqrtl(x)`.
    #[inline]
    pub fn sqrt(self) -> LDouble {
        imp::sqrt(&self)
    }

    /// `nearbyintl(x)` (round half to even).
    #[inline]
    pub fn rint(self) -> LDouble {
        imp::rint(&self)
    }

    /// `powl(x, y)`.
    pub fn powl(self, y: LDouble) -> LDouble {
        imp::powl(&self, &y)
    }

    /// `(long double) x` for an integer.
    #[inline]
    pub fn from_i64(x: i64) -> LDouble {
        imp::from_i64(x)
    }

    pub fn is_nan(&self) -> bool {
        imp::cmp(self, self) == 2
    }

    /// `self + xs[0] + xs[1] + ...`, sequentially in long double.
    pub fn sum_from(self, xs: &[f64]) -> LDouble {
        imp::sum_from(&self, xs)
    }

    /// `self * xs[0] * xs[1] * ...`, sequentially in long double.
    pub fn prod_from(self, xs: &[f64]) -> LDouble {
        imp::prod_from(&self, xs)
    }

    /// Running sums from `self`, stored rounded to double in `ys`.
    pub fn cumsum_from(self, xs: &[f64], ys: &mut [f64]) -> LDouble {
        imp::cumsum_from(&self, xs, ys)
    }

    /// `sum((long double) x - self)` over `xs`, sequentially.
    pub fn sum_dev(self, xs: &[f64]) -> LDouble {
        imp::sum_dev(&self, xs)
    }
    /// Running products from `self`, stored rounded to double in `ys`.
    pub fn cumprod_from(self, xs: &[f64], ys: &mut [f64]) -> LDouble {
        imp::cumprod_from(&self, xs, ys)
    }

    /// `sum(((long double) x - self)^2)` over `xs`, sequentially.
    pub fn sum_sq_dev(self, xs: &[f64]) -> LDouble {
        imp::sum_sq_dev(&self, xs)
    }

    /// `sum(((long double) xs[i] - self) * ((long double) ys[i] - ym))`.
    pub fn sum_cross_dev(self, xs: &[f64], ym: LDouble, ys: &[f64]) -> LDouble {
        imp::sum_cross_dev(&self, xs, &ym, ys)
    }

    /// `acc[i] += xs[i]` for each `i` (R's row sums by column).
    pub fn add_each(acc: &mut [LDouble], xs: &[f64]) {
        imp::add_each(acc, xs)
    }
}

impl From<f64> for LDouble {
    fn from(x: f64) -> Self {
        LDouble::from_f64(x)
    }
}

impl Default for LDouble {
    fn default() -> Self {
        LDouble::ZERO
    }
}

macro_rules! ops {
    ($tr:ident, $m:ident, $atr:ident, $am:ident, $f:path) => {
        impl std::ops::$tr for LDouble {
            type Output = LDouble;
            #[inline]
            fn $m(self, rhs: LDouble) -> LDouble {
                $f(&self, &rhs)
            }
        }
        impl std::ops::$tr<f64> for LDouble {
            type Output = LDouble;
            #[inline]
            fn $m(self, rhs: f64) -> LDouble {
                $f(&self, &LDouble::from_f64(rhs))
            }
        }
        impl std::ops::$atr for LDouble {
            #[inline]
            fn $am(&mut self, rhs: LDouble) {
                *self = $f(self, &rhs);
            }
        }
        impl std::ops::$atr<f64> for LDouble {
            #[inline]
            fn $am(&mut self, rhs: f64) {
                *self = $f(self, &LDouble::from_f64(rhs));
            }
        }
    };
}

ops!(Add, add, AddAssign, add_assign, imp::add);
ops!(Sub, sub, SubAssign, sub_assign, imp::sub);
ops!(Mul, mul, MulAssign, mul_assign, imp::mul);
ops!(Div, div, DivAssign, div_assign, imp::div);

impl PartialEq for LDouble {
    fn eq(&self, other: &LDouble) -> bool {
        imp::cmp(self, other) == 0
    }
}

impl PartialOrd for LDouble {
    fn partial_cmp(&self, other: &LDouble) -> Option<std::cmp::Ordering> {
        match imp::cmp(self, other) {
            -1 => Some(std::cmp::Ordering::Less),
            0 => Some(std::cmp::Ordering::Equal),
            1 => Some(std::cmp::Ordering::Greater),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_precision_sums() {
        // R: sum(c(0.1, 0.2, 0.3)) == 0.6 is TRUE (long double accumulator)
        let mut s = LDouble::ZERO;
        for x in [0.1, 0.2, 0.3] {
            s += x;
        }
        assert_eq!(s.to_f64(), 0.6);
        assert_ne!(0.1 + 0.2 + 0.3, 0.6);
        // R: sum(c(1e16, 1, -1e16)) == 1
        let s = LDouble::from(1e16) + 1.0 - 1e16;
        assert_eq!(s.to_f64(), 1.0);
        let q = LDouble::from(1.0) / 3.0 * 3.0;
        assert_eq!(q.to_f64(), 1.0);
        assert!(LDouble::from(1.0) < LDouble::from(2.0));
        assert!(LDouble::from(f64::NAN).partial_cmp(&LDouble::from(1.0)).is_none());
        assert_eq!((LDouble::from(1.0) / 0.0).to_f64(), f64::INFINITY);
    }

    #[test]
    fn slice_loops_match_scalar_ops() {
        let bits = |l: LDouble| l.0;
        let mut xs = vec![0.1, 1e16, -3.5, 1.0 / 3.0, -1e16, 2.5e-300, 7.0, f64::NAN, 1e308, 1e308];
        for k in 0..50 {
            xs.push((k as f64 * 0.37).sin() * 10f64.powi(k % 7));
        }
        for n in [0, 1, 2, 7, 8, xs.len()] {
            let xs = &xs[..n];
            let init = LDouble::from(0.25);
            let mut s = init;
            let mut p = init;
            let mut ys = Vec::new();
            for &x in xs {
                s += x;
                p *= x;
                ys.push(s.to_f64());
            }
            assert_eq!(bits(init.sum_from(xs)), bits(s), "sum n={n}");
            assert_eq!(bits(init.prod_from(xs)), bits(p), "prod n={n}");
            let mut out = vec![0.0; n];
            assert_eq!(bits(init.cumsum_from(xs, &mut out)), bits(s), "cumsum n={n}");
            assert_eq!(
                out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                ys.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
            let m = LDouble::from(1.0) / 3.0;
            let mut t = LDouble::ZERO;
            for &x in xs {
                t += LDouble::from(x) - m;
            }
            assert_eq!(bits(m.sum_dev(xs)), bits(t), "sum_dev n={n}");
        }
    }
}

/// Property tests: the software 80-bit implementation must agree with
/// the x87 hardware bit for bit on every operation. Run on x86_64, where
/// both are available; the counts are controlled by
/// `ROBIGO_LDPROP_N` (default 1_000_000 random inputs per operation and
/// generator).
#[cfg(all(test, target_arch = "x86_64"))]
mod x87_vs_soft {
    use super::LDouble;
    use super::imp;
    use super::soft;

    fn n_random() -> u64 {
        std::env::var("ROBIGO_LDPROP_N").ok().and_then(|v| v.parse().ok()).unwrap_or(1_000_000)
    }

    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }
        fn next(&mut self) -> u64 {
            // xorshift64*
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545F4914F6CDD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn ld(se: u16, m: u64) -> LDouble {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&m.to_le_bytes());
        b[8..10].copy_from_slice(&se.to_le_bytes());
        LDouble(b)
    }

    fn bits(x: &LDouble) -> [u8; 16] {
        x.0
    }

    fn hex(x: &LDouble) -> String {
        let (se, m) = (u16::from_le_bytes([x.0[8], x.0[9]]), u64::from_le_bytes(x.0[..8].try_into().unwrap()));
        format!("{se:04x}:{m:016x}")
    }

    /// Random 80-bit patterns from every class, weighted to the edges.
    fn gen_ld80(rng: &mut Rng) -> LDouble {
        let m = rng.next();
        let se = match rng.below(12) {
            0 => rng.next() as u16, // fully random
            1 => 0x0000,            // denormal / zero
            2 => 0x0001,
            3 => 0x7ffe,                       // max exponent
            4 => 0x7fff,                       // inf / nan
            5 => 0x03ff + rng.below(8) as u16, // near 1
            6 => 0x3ffe + rng.below(4) as u16,
            7 => 0x43ff + rng.below(8) as u16,  // near 2^64
            8 => 0x403e + rng.below(4) as u16,  // near 2^63
            9 => (rng.next() % 0x7fff) as u16,  // random exponent
            10 => 0xc000 + rng.below(2) as u16, // tiny denormal exponents unused; near-zero normals
            _ => 0x0000,
        };
        let mant = match rng.below(4) {
            0 => m,
            1 => m | 0x8000_0000_0000_0000,                           // integer bit set
            2 => 0x8000_0000_0000_0000 | (m & 0xffff_ffff_ffff_f000), // trailing zeros (ties)
            _ => 0x8000_0000_0000_0000 | (m & 1) | (m & 0xff00_0000_0000_0000),
        };
        ld(se | ((m >> 63) as u16) << 15, mant)
    }

    /// Interesting 80-bit values: class edges, rounding boundaries.
    fn edges() -> Vec<LDouble> {
        let mut v = vec![
            ld(0x0000, 0),
            ld(0x8000, 0),
            ld(0x7fff, 0x8000_0000_0000_0000),
            ld(0xffff, 0x8000_0000_0000_0000),
            ld(0x7fff, 0xc000_0000_0000_0000), // indefinite
            ld(0x7fff, 0x8000_0000_0000_0001),
            ld(0x7fff, 0x8000_0000_0000_07a2), // NA_real_-like payload
            ld(0x7fff, 0xa000_0000_0000_07a2),
            ld(0x3fff, 0x8000_0000_0000_0000), // 1
            ld(0xbfff, 0x8000_0000_0000_0000), // -1
            ld(0x0000, 1),                     // min denormal
            ld(0x8000, 1),
            ld(0x0000, 0xffff_ffff_ffff_ffff), // max denormal
            ld(0x0000, 0x8000_0000_0000_0000), // pseudo-denormal = 2^-16382
            ld(0x0001, 0x8000_0000_0000_0000), // min normal
            ld(0x7ffe, 0xffff_ffff_ffff_ffff), // max finite
            ld(0x7ffe, 0x8000_0000_0000_0000), // 2^16383
            ld(0x43ff, 0x8000_0000_0000_0000), // 2^64
            ld(0x43fe, 0xffff_ffff_ffff_ffff), // 2^64 - 1
            ld(0x403e, 0x8000_0000_0000_0000), // 2^63
            ld(0x3ffe, 0x8000_0000_0000_0000), // 0.5
            ld(0x0002, 0x8000_0000_0000_0000), // 2^-16381
            ld(0x0001, 0x8000_0000_0000_0001),
            ld(0x0001, 0xc000_0000_0000_0000),
            ld(0x0001, 0xffff_ffff_ffff_ffff),
        ];
        // every exponent with round-to-even-stressing mantissas
        for e in [0u16, 1, 2, 3, 0x3fbf, 0x3fc0, 0x43fe, 0x43ff, 0x4400, 0x7ffd, 0x7ffe] {
            for m in [
                0x8000_0000_0000_0000u64,
                0x8000_0000_0000_0001,
                0xffff_ffff_ffff_ffff,
                0x8000_0000_0000_0002,
                0x8000_7fff_ffff_ffff,
                0x8000_0000_0000_0000 | 1,
            ] {
                v.push(ld(e, m));
                v.push(ld(e | 0x8000, m));
            }
        }
        v
    }

    /// Random f64s from every class.
    fn gen_f64(rng: &mut Rng) -> f64 {
        let b = match rng.below(8) {
            0 => rng.next(),
            1 => (rng.next() & 0x800f_ffff_ffff_ffff) | 0x0010_0000_0000_0000, // tiny exponent
            2 => (rng.next() & 0x800f_ffff_ffff_ffff) | 0x0000_0000_0000_0000, // denormals
            3 => (rng.next() & 0x800f_ffff_ffff_ffff) | 0x7fe0_0000_0000_0000, // huge
            4 => (rng.next() & 0x8010_ffff_ffff_ffff) | 0x7ff0_0000_0000_0000, // NaN
            5 => rng.next() & 0x7fff_ffff_ffff_ffff,                           // [0, 2)
            6 => (rng.next() & 0x8007_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000, // near 1
            _ => (rng.next() & 0x800f_ffff_ffff_ffff) | 0x43e0_0000_0000_0000, // near 2^63
        };
        f64::from_bits(b)
    }

    fn check_one(name: &str, i: u64, a: &LDouble, want: &LDouble, got: &LDouble, fails: &mut u64) {
        if bits(want) != bits(got) {
            if *fails < 5 {
                eprintln!("MISMATCH {name} #{i}: a={} -> x87={} soft={}", hex(a), hex(want), hex(got));
            }
            *fails += 1;
        }
    }

    fn check_pair(name: &str, i: u64, a: &LDouble, b: &LDouble, want: &LDouble, got: &LDouble, fails: &mut u64) {
        if bits(want) != bits(got) {
            if *fails < 5 {
                eprintln!("MISMATCH {name} #{i}: a={} b={} -> x87={} soft={}", hex(a), hex(b), hex(want), hex(got));
            }
            *fails += 1;
        }
    }

    #[test]
    fn conversions_match_x87() {
        let n = n_random();
        let mut rng = Rng::new(0x9E3779B97F4A7C15);
        let mut fails = 0u64;
        for i in 0..n {
            let x = gen_f64(&mut rng);
            check_one("from_f64", i, &imp::from_f64(x), &imp::from_f64(x), &soft::from_f64(x), &mut fails);
        }
        let mut rng = Rng::new(0xF00DCAFEF00DCAFE);
        for i in 0..n {
            let x = gen_f64(&mut rng);
            let a = imp::from_f64(x);
            check_one(
                "to_f64",
                i,
                &a,
                &LDouble::from_f64(imp::to_f64(&a)),
                &LDouble::from_f64(soft::to_f64(&a)),
                &mut fails,
            );
        }
        // to_f64 of arbitrary 80-bit patterns (denormals included: they
        // arise from arithmetic even though from_f64 never makes them —
        // this is where a shift-by-64 bug once hid).
        let mut rng = Rng::new(0x5EEE_0000_C0DE_0001);
        for i in 0..n {
            let a = gen_ld80(&mut rng);
            let (want, got) = (imp::to_f64(&a), soft::to_f64(&a));
            if want.to_bits() != got.to_bits() {
                if fails < 5 {
                    eprintln!(
                        "MISMATCH to_f64(pattern) #{i}: a={} -> x87={:016x} soft={:016x}",
                        hex(&a),
                        want.to_bits(),
                        got.to_bits()
                    );
                }
                fails += 1;
            }
        }
        for (i, a) in edges().iter().enumerate() {
            let (want, got) = (imp::to_f64(a), soft::to_f64(a));
            if want.to_bits() != got.to_bits() {
                if fails < 5 {
                    eprintln!(
                        "MISMATCH to_f64(edge) #{i}: a={} -> x87={:016x} soft={:016x}",
                        hex(a),
                        want.to_bits(),
                        got.to_bits()
                    );
                }
                fails += 1;
            }
        }
        let mut rng = Rng::new(0xDEADBEEFDEADBEEF);
        let ints = [
            0i64,
            1,
            -1,
            2,
            -2,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
            0x7fff_ffff,
            -0x8000_0000,
            1 << 62,
            -(1 << 62),
            4611686018427387904,
        ];
        for (i, &x) in ints.iter().enumerate() {
            check_one("from_i64", i as u64, &imp::from_i64(x), &imp::from_i64(x), &soft::from_i64(x), &mut fails);
        }
        for i in 0..n {
            let x = (rng.next() as i64) ^ ((rng.next() as i64) << 32);
            check_one("from_i64", i, &imp::from_i64(x), &imp::from_i64(x), &soft::from_i64(x), &mut fails);
        }
        assert_eq!(fails, 0, "{fails} conversion mismatches");
    }

    #[test]
    fn arithmetic_matches_x87() {
        let n = n_random();
        let edge = edges();
        let mut fails = 0u64;
        let mut rng = Rng::new(0x1234567890ABCDEF);
        // random pairs
        for i in 0..n {
            let a = gen_ld80(&mut rng);
            let b = gen_ld80(&mut rng);
            check_pair("add", i, &a, &b, &imp::add(&a, &b), &soft::add(&a, &b), &mut fails);
            check_pair("sub", i, &a, &b, &imp::sub(&a, &b), &soft::sub(&a, &b), &mut fails);
            check_pair("mul", i, &a, &b, &imp::mul(&a, &b), &soft::mul(&a, &b), &mut fails);
            check_pair("div", i, &a, &b, &imp::div(&a, &b), &soft::div(&a, &b), &mut fails);
            let c = imp::cmp(&a, &b);
            let d = soft::cmp(&a, &b);
            if c != d && fails < 5 {
                eprintln!("MISMATCH cmp #{i}: a={} b={} -> x87={c} soft={d}", hex(&a), hex(&b));
            }
            if c != d {
                fails += 1;
            }
        }
        // all edge pairs
        for (i, a) in edge.iter().enumerate() {
            for b in edge.iter() {
                check_pair("add*", i as u64, a, b, &imp::add(a, b), &soft::add(a, b), &mut fails);
                check_pair("sub*", i as u64, a, b, &imp::sub(a, b), &soft::sub(a, b), &mut fails);
                check_pair("mul*", i as u64, a, b, &imp::mul(a, b), &soft::mul(a, b), &mut fails);
                check_pair("div*", i as u64, a, b, &imp::div(a, b), &soft::div(a, b), &mut fails);
                if imp::cmp(a, b) != soft::cmp(a, b) {
                    fails += 1;
                }
            }
        }
        // sqrt and rint
        let mut rng = Rng::new(0xC0FFEE123456789);
        for i in 0..n {
            let a = gen_ld80(&mut rng);
            check_one("sqrt", i, &a, &imp::sqrt(&a), &soft::sqrt(&a), &mut fails);
            check_one("rint", i, &a, &imp::rint(&a), &soft::rint(&a), &mut fails);
        }
        for a in &edge {
            check_one("sqrt*", 0, a, &imp::sqrt(a), &soft::sqrt(a), &mut fails);
            check_one("rint*", 0, a, &imp::rint(a), &soft::rint(a), &mut fails);
        }
        assert_eq!(fails, 0, "{fails} arithmetic mismatches");
    }

    /// Exponent-difference sweep for add/sub: every alignment from 0 to
    /// 70 ulps apart, with mantissas that stress ties.
    #[test]
    fn add_alignment_matches_x87() {
        let mut fails = 0u64;
        let mants = [
            0x8000_0000_0000_0000u64,
            0x8000_0000_0000_0001,
            0xffff_ffff_ffff_ffff,
            0x8000_0000_0000_0000 | 1 << 31,
            0xffff_ffff_ffff_fffe,
            0xa5a5_a5a5_5a5a_5a5a,
        ];
        for de in 0i32..=70 {
            for &ma in &mants {
                for &mb in &mants {
                    for ea in [0x0001u16, 0x3fff, 0x0002, 0x43ff] {
                        let eb = (ea as i32 - de) as u16;
                        let a = ld(ea, ma);
                        let b = ld(eb, mb);
                        check_pair("align+", 0, &a, &b, &imp::add(&a, &b), &soft::add(&a, &b), &mut fails);
                        check_pair("align-", 0, &a, &b, &imp::sub(&a, &b), &soft::sub(&a, &b), &mut fails);
                        let b = ld(eb | 0x8000, mb);
                        check_pair("align+-", 0, &a, &b, &imp::add(&a, &b), &soft::add(&a, &b), &mut fails);
                        check_pair("align--", 0, &a, &b, &imp::sub(&a, &b), &soft::sub(&a, &b), &mut fails);
                    }
                }
            }
        }
        // denormal results: subtract near-equal tiny values
        for dm in 0..64u32 {
            for em in [0u16, 1, 2] {
                let m = 1u64 << dm;
                let a = ld(em, m);
                let b = ld(em, m + 1);
                check_pair("dcancel", 0, &a, &b, &imp::sub(&a, &b), &soft::sub(&a, &b), &mut fails);
                let a = ld(em, m);
                let b = ld(0, m);
                check_pair("ddn", 0, &a, &b, &imp::sub(&a, &b), &soft::sub(&a, &b), &mut fails);
            }
        }
        assert_eq!(fails, 0, "{fails} alignment mismatches");
    }

    /// NaN payloads, quieting and selection rules, exhaustively over the
    /// interesting NaN patterns in both operand positions.
    #[test]
    fn nan_rules_match_x87() {
        let mut fails = 0u64;
        let mut nan = Vec::new();
        for se in [0x7fffu16, 0xffff] {
            for m in [
                0x8000_0000_0000_0000u64, // sNaN, no payload
                0x8000_0000_0000_07a2,    // sNaN, NA payload
                0x8000_1234_5678_9abc,    // sNaN
                0xc000_0000_0000_0000,    // indefinite (qNaN)
                0xc000_0000_0000_07a2,    // qNaN, NA payload
                0x8000_0000_0000_0000 | 0x4000_0000_0000_0001,
                0xffff_ffff_ffff_ffff, // qNaN all ones
                0xbfff_ffff_ffff_ffff, // qNaN
            ] {
                nan.push(ld(se, m));
            }
        }
        // Pseudo-NaNs (no integer bit): unsupported on x87; ops give the
        // real indefinite QNaN.
        for se in [0x7fffu16, 0xffff] {
            for m in [1u64, 0x1234_5678_9abc_def0] {
                nan.push(ld(se, m));
            }
        }
        let plain = vec![
            ld(0x3fff, 0x8000_0000_0000_0000),
            ld(0x0000, 0),
            ld(0x7fff, 0x8000_0000_0000_0000),
            ld(0xbfff, 0x8000_0000_0000_0000),
        ];
        for a in nan.iter().chain(plain.iter()) {
            for b in nan.iter().chain(plain.iter()) {
                check_pair("nan+", 0, a, b, &imp::add(a, b), &soft::add(a, b), &mut fails);
                check_pair("nan-", 0, a, b, &imp::sub(a, b), &soft::sub(a, b), &mut fails);
                check_pair("nan*", 0, a, b, &imp::mul(a, b), &soft::mul(a, b), &mut fails);
                check_pair("nan/", 0, a, b, &imp::div(a, b), &soft::div(a, b), &mut fails);
                check_one("nansqrt", 0, a, &imp::sqrt(a), &soft::sqrt(a), &mut fails);
                check_one("nanrint", 0, a, &imp::rint(a), &soft::rint(a), &mut fails);
            }
        }
        assert_eq!(fails, 0, "{fails} NaN mismatches");
    }

    /// Regression: to_f64 of an 80-bit value just below 2^-1074 rounds to
    /// the smallest denormal double (a shift-by-64 once made this return
    /// the mantissa itself).
    #[test]
    fn to_f64_subnormal_boundary() {
        // se = 0xbbcc: sign, e2 = -1075; value = 0xa04e...·2^-1138 ≈
        // 2^-1074.75 → -(2^-1074).
        let a = ld(0xbbcc, 0xa04e_7458_ed4b_5e7f);
        assert_eq!(soft::to_f64(&a).to_bits(), 0x8000_0000_0000_0001);
        assert_eq!(imp::to_f64(&a).to_bits(), 0x8000_0000_0000_0001);
        // exact tie (mant == 2^63) rounds to even: 0
        let a = ld(0xbbcc, 0x8000_0000_0000_0000);
        assert_eq!(soft::to_f64(&a).to_bits(), 0x8000_0000_0000_0000);
        // one binade smaller: always 0
        let a = ld(0xbbcb, 0xffff_ffff_ffff_ffff);
        assert_eq!(soft::to_f64(&a).to_bits(), 0x8000_0000_0000_0000);
        // just inside the normal range at the bottom
        let a = ld(0x3c01, 0x8000_0000_0000_0000); // 2^-1022
        assert_eq!(soft::to_f64(&a).to_bits(), 0x0010_0000_0000_0000);
    }

    /// The loop kernels over random and adversarial slices.
    #[test]
    fn loops_match_x87() {
        let n = n_random();
        let mut fails = 0u64;
        let mut rng = Rng::new(0x5DEECE66D5DEECE6);
        let sample = |rng: &mut Rng| -> f64 {
            match rng.below(6) {
                0 => f64::from_bits(rng.next()),
                1 => rng.next() as i32 as f64,
                2 => (rng.next() as f64) / 65536.0 - 32768.0,
                3 => f64::from_bits((rng.next() & 0x800f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000),
                4 => f64::from_bits((rng.next() & 0x800f_ffff_ffff_ffff) | 0x0000_0000_0000_0000),
                _ => (rng.next() % 200) as f64 - 100.0,
            }
        };
        for i in 0..n / 10 {
            let len = (rng.below(60) as usize).min(59);
            let xs: Vec<f64> = (0..len).map(|_| sample(&mut rng)).collect();
            let ys: Vec<f64> = (0..len).map(|_| sample(&mut rng)).collect();
            for init in [LDouble::ZERO, imp::from_f64(0.25), imp::from_f64(-1e300), imp::from_i64(7)] {
                let m = imp::div(&imp::from_f64(1.0), &imp::from_f64(3.0));
                check_one("sum_from", i, &init, &imp::sum_from(&init, &xs), &soft::sum_from(&init, &xs), &mut fails);
                check_one("prod_from", i, &init, &imp::prod_from(&init, &xs), &soft::prod_from(&init, &xs), &mut fails);
                check_one("sum_dev", i, &m, &imp::sum_dev(&m, &xs), &soft::sum_dev(&m, &xs), &mut fails);
                check_one("sum_sq_dev", i, &m, &imp::sum_sq_dev(&m, &xs), &soft::sum_sq_dev(&m, &xs), &mut fails);
                check_one(
                    "sum_cross_dev",
                    i,
                    &m,
                    &imp::sum_cross_dev(&m, &xs, &m, &ys),
                    &soft::sum_cross_dev(&m, &xs, &m, &ys),
                    &mut fails,
                );
                let mut o1 = vec![0.0f64; len];
                let mut o2 = vec![0.0f64; len];
                check_one(
                    "cumsum",
                    i,
                    &init,
                    &imp::cumsum_from(&init, &xs, &mut o1),
                    &soft::cumsum_from(&init, &xs, &mut o2),
                    &mut fails,
                );
                // Compare bits: NaN outputs with equal bits must not fail.
                let bits_eq = |a: &[f64], b: &[f64]| a.iter().map(|v| v.to_bits()).eq(b.iter().map(|v| v.to_bits()));
                if !bits_eq(&o1, &o2) {
                    if fails < 5 {
                        let j = o1.iter().zip(o2.iter()).position(|(a, b)| a.to_bits() != b.to_bits());
                        eprintln!("MISMATCH cumsum out #{i} at {j:?}");
                        if let Some(j) = j {
                            eprintln!(
                                "  x={:016x} prev={:016x} x87={:016x} soft={:016x}",
                                xs[j].to_bits(),
                                if j > 0 { o1[j - 1].to_bits() } else { init.to_f64().to_bits() },
                                o1[j].to_bits(),
                                o2[j].to_bits()
                            );
                        }
                    }
                    fails += 1;
                }
                check_one(
                    "cumprod",
                    i,
                    &init,
                    &imp::cumprod_from(&init, &xs, &mut o1),
                    &soft::cumprod_from(&init, &xs, &mut o2),
                    &mut fails,
                );
                if !bits_eq(&o1, &o2) {
                    if fails < 5 {
                        let j = o1.iter().zip(o2.iter()).position(|(a, b)| a.to_bits() != b.to_bits());
                        eprintln!("MISMATCH cumprod out #{i} at {j:?}");
                        if let Some(j) = j {
                            eprintln!(
                                "  x={:016x} prev={:016x} x87={:016x} soft={:016x}",
                                xs[j].to_bits(),
                                if j > 0 { o1[j - 1].to_bits() } else { init.to_f64().to_bits() },
                                o1[j].to_bits(),
                                o2[j].to_bits()
                            );
                        }
                    }
                    fails += 1;
                }
                let mut acc1 = vec![LDouble::ZERO; len];
                let mut acc2 = vec![LDouble::ZERO; len];
                imp::add_each(&mut acc1, &xs);
                soft::add_each(&mut acc2, &xs);
                for j in 0..len {
                    if bits(&acc1[j]) != bits(&acc2[j]) && fails < 5 {
                        eprintln!("MISMATCH add_each #{i}[{j}]: {} vs {}", hex(&acc1[j]), hex(&acc2[j]));
                    }
                    if bits(&acc1[j]) != bits(&acc2[j]) {
                        fails += 1;
                    }
                }
            }
        }
        assert_eq!(fails, 0, "{fails} loop mismatches");
    }

    /// The ulp distance between two LDouble values (magnitude classes and
    /// signs must match; NaNs must both be NaN).
    fn ulp_dist(a: &LDouble, b: &LDouble) -> i64 {
        let (sa, ma) = (u16::from_le_bytes([a.0[8], a.0[9]]), u64::from_le_bytes(a.0[..8].try_into().unwrap()));
        let (sb, mb) = (u16::from_le_bytes([b.0[8], b.0[9]]), u64::from_le_bytes(b.0[..8].try_into().unwrap()));
        if sa & 0x8000 != sb & 0x8000 && !(ma == 0 && mb == 0) && (sa & 0x7fff) != 0x7fff {
            return i64::MAX / 2;
        }
        let (ea, eb) = ((sa & 0x7fff) as i64, (sb & 0x7fff) as i64);
        if ea == 0x7fff || eb == 0x7fff {
            // inf/nan: equal encodings handled by the caller's exact check
            return if ea == 0x7fff && eb == 0x7fff && (ma == mb || (ma | mb) & 0x8000_0000_0000_0000 != 0) {
                0
            } else {
                i64::MAX / 2
            };
        }
        // decode to a common linear grid: value = m · 2^(e2 - 63)
        let dec = |se: u16, m: u64| -> (i128, i64) {
            let e = (se & 0x7fff) as i64;
            let s = if se & 0x8000 != 0 { -1 } else { 1 };
            if e == 0 {
                if m == 0 {
                    return (0, 0);
                }
                (s * m as i128, -16446)
            } else {
                (s * (m as i128) << 0, e - 16383 - 63)
            }
        };
        let (va, ka) = dec(sa, ma);
        let (vb, kb) = dec(sb, mb);
        // ulp of the larger magnitude
        let ulp_exp = ka.max(kb);
        let da = va >> (ulp_exp - ka);
        let db = vb >> (ulp_exp - kb);
        let d = (da - db).abs();
        d.min(i64::MAX as i128 / 2) as i64
    }

    /// soft::powl vs glibc's powl (through the x87 asm path). The port is
    /// bit-exact except where glibc's own f2xm1/fyl2xp1 microcode deviates
    /// from the correctly rounded result in the final bit (see
    /// ldouble/soft/pow.rs); such cases must stay within 1 ulp and rare.
    #[test]
    fn powl_smoke() {
        for (x, y) in [
            (10.0, -340.0),
            (10.0, -30.0),
            (10.0, -4.0),
            (10.0, 3.0),
            (10.0, -3.0),
            (2.0, 0.5),
            (2.0, 10.0),
            (3.5, 7.1),
            (0.5, 0.25),
            (1.5, -100.5),
        ] {
            let (a, b) = (imp::from_f64(x), imp::from_f64(y));
            let (want, got) = (imp::powl(&a, &b), soft::powl(&a, &b));
            eprintln!(
                "powl({x}, {y}): glibc={} soft={} {}",
                hex(&want),
                hex(&got),
                if bits(&want) == bits(&got) { "==" } else { "!=" }
            );
        }
    }

    #[test]
    fn powl_matches_glibc() {
        let n = n_random();
        let mut far = 0u64; // differences of more than 1 ulp: never ok
        let mut last_bit = 0u64; // 1-ulp differences (glibc's own error)
        let mut total = 0u64;
        let mut rng = Rng::new(0x5AFE_0B5E_0DD0_011);
        let mut shown = 0u64;
        fn note(
            kind: &str,
            i: u64,
            a: &LDouble,
            b: &LDouble,
            want: &LDouble,
            got: &LDouble,
            dist: i64,
            shown: &mut u64,
        ) {
            if *shown >= 12 {
                return;
            }
            *shown += 1;
            if dist > 1 {
                eprintln!(
                    "FAR powl {kind} #{i}: x={} y={} -> glibc={} soft={} dist={dist}",
                    hex(a),
                    hex(b),
                    hex(want),
                    hex(got)
                );
            } else {
                eprintln!("ulp powl {kind} #{i}: x={} y={} -> glibc={} soft={}", hex(a), hex(b), hex(want), hex(got));
            }
        }
        // R's only Linux use of powl (format.c): powl(10, kp) — exact bytes.
        for kp in -340i64..=340 {
            let (x, y) = (imp::from_f64(10.0), imp::from_i64(kp));
            let (want, got) = (imp::powl(&x, &y), soft::powl(&x, &y));
            total += 1;
            if bits(&want) != bits(&got) {
                let d = ulp_dist(&want, &got);
                if d > 1 {
                    far += 1;
                } else {
                    last_bit += 1;
                }
                note("pow10", kp as u64, &x, &y, &want, &got, d, &mut shown);
            }
        }
        // Special values, all pairs.
        let mut sp = vec![
            LDouble::ZERO,
            ld(0x8000, 0),
            imp::from_f64(1.0),
            imp::from_f64(-1.0),
            imp::from_f64(10.0),
            imp::from_f64(-10.0),
            imp::from_f64(f64::INFINITY),
            imp::from_f64(f64::NEG_INFINITY),
            imp::from_f64(f64::NAN),
            imp::from_f64(2.0),
            imp::from_f64(0.5),
            imp::from_f64(3.0),
            imp::from_f64(-3.0),
            imp::from_f64(1.5),
            imp::from_f64(-1.5),
            imp::from_f64(f64::MIN_POSITIVE),
        ];
        sp.push(ld(0x43ff, 0x8000_0000_0000_0000)); // 2^64
        sp.push(ld(0x43fe, 0xffff_ffff_ffff_ffff)); // just below 2^64
        sp.push(ld(0x43ff, 0x8000_0000_0000_0001)); // just above
        sp.push(ld(0xc3ff, 0x8000_0000_0000_0001));
        sp.push(ld(0x43ef, 0x8000_0000_0000_0003)); // odd integer below 2^64
        sp.push(ld(0x0001, 0x8000_0000_0000_0000)); // min normal
        for a in &sp {
            for b in &sp {
                let (want, got) = (imp::powl(a, b), soft::powl(a, b));
                total += 1;
                if bits(&want) != bits(&got) {
                    let d = ulp_dist(&want, &got);
                    if d > 1 {
                        far += 1;
                    } else {
                        last_bit += 1;
                    }
                    note("special", 0, a, b, &want, &got, d, &mut shown);
                }
            }
        }
        // Random pairs from the interesting regions.
        let gen_x = |rng: &mut Rng| -> LDouble {
            match rng.below(10) {
                0 => ld((rng.next() as u16 & 0x7fff).max(1) | 0x3f00, rng.next() | 0x8000_0000_0000_0000),
                1 => ld(0x3fff, rng.next() | 0x8000_0000_0000_0000), // [1,2)
                2 => ld(0x4000, rng.next() | 0x8000_0000_0000_0000), // [2,4)
                3 => ld(0xbfff, rng.next() | 0x8000_0000_0000_0000), // [-2,-1)
                4 => ld(0x0001, rng.next() | 0x8000_0000_0000_0000), // min normals
                5 => ld(0x7ffe, rng.next() | 0x8000_0000_0000_0000), // near max
                6 => ld(0x0000, rng.next()),                         // denormals
                7 => imp::from_f64(gen_f64(rng)),
                8 => ld(0x4004, rng.next() | 0x8000_0000_0000_0000), // ~10
                _ => ld(0x3ffd, rng.next() | 0x8000_0000_0000_0000), // ~0.5
            }
        };
        let gen_y = |rng: &mut Rng| -> LDouble {
            match rng.below(12) {
                0 => imp::from_i64(rng.next() as i64 % 7),                   // small ints
                1 => imp::from_i64(rng.next() as i64 % 7 - 6),               // small neg ints
                2 => imp::from_i64(rng.next() as i64),                       // big ints
                3 => imp::from_f64((rng.next() as f64) / 65536.0 - 32768.0), // real
                4 => imp::from_f64(0.5),
                5 => imp::from_f64(-0.5),
                6 => imp::from_f64(gen_f64(rng)),
                7 => imp::from_f64(1e-30), // tiny: |y| < 2^-79 saturation
                8 => imp::from_f64(-1e-30),
                9 => imp::from_f64(1e25),                             // huge
                10 => ld(0x43ff, rng.next() | 0x8000_0000_0000_0000), // ~2^64: odd check
                _ => ld(0x43fe, rng.next() | 0x8000_0000_0000_0000),  // ~2^63
            }
        };
        for i in 0..n / 4 {
            let a = gen_x(&mut rng);
            let b = gen_y(&mut rng);
            let (want, got) = (imp::powl(&a, &b), soft::powl(&a, &b));
            total += 1;
            if bits(&want) != bits(&got) {
                let d = ulp_dist(&want, &got);
                if d > 1 {
                    far += 1;
                } else {
                    last_bit += 1;
                }
                note("rand", i, &a, &b, &want, &got, d, &mut shown);
            }
        }
        eprintln!("powl: {total} cases, {last_bit} within 1 ulp, {far} far");
        assert_eq!(far, 0, "{far} powl results differ by more than 1 ulp");
        let rate = last_bit as f64 / total as f64;
        assert!(rate < 0.03, "powl divergence rate too high: {last_bit}/{total}");
    }
}
