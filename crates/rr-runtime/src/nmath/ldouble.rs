//! C `long double` (R's `LDOUBLE`) as GNU R uses it on x86_64: the x87
//! 80-bit extended format with a 64-bit significand, rounding to nearest.
//!
//! R accumulates in long double in several places (`sum()`, `mean()`,
//! `cumsum()` in summary.c/cum.c; pdhyper, pnchisq, pnbeta, pnt, ptukey
//! in nmath), so plain `f64` arithmetic can differ in the last bit.
//! On x86_64 the operations below run on the x87 FPU, which gives the
//! same results as the C code. Elsewhere they fall back to `f64`.

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

#[cfg(not(target_arch = "x86_64"))]
mod imp {
    use super::LDouble;
    fn get(x: &LDouble) -> f64 {
        f64::from_le_bytes(x.0[..8].try_into().unwrap())
    }
    fn put(v: f64) -> LDouble {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&v.to_le_bytes());
        LDouble(b)
    }
    pub fn from_f64(x: f64) -> LDouble {
        put(x)
    }
    pub fn from_i64(x: i64) -> LDouble {
        put(x as f64)
    }
    pub fn to_f64(x: &LDouble) -> f64 {
        get(x)
    }
    pub fn add(a: &LDouble, b: &LDouble) -> LDouble {
        put(get(a) + get(b))
    }
    pub fn sub(a: &LDouble, b: &LDouble) -> LDouble {
        put(get(a) - get(b))
    }
    pub fn mul(a: &LDouble, b: &LDouble) -> LDouble {
        put(get(a) * get(b))
    }
    pub fn div(a: &LDouble, b: &LDouble) -> LDouble {
        put(get(a) / get(b))
    }
    pub fn sqrt(a: &LDouble) -> LDouble {
        put(get(a).sqrt())
    }
    pub fn sum_from(init: &LDouble, xs: &[f64]) -> LDouble {
        put(xs.iter().fold(get(init), |s, x| s + x))
    }
    pub fn prod_from(init: &LDouble, xs: &[f64]) -> LDouble {
        put(xs.iter().fold(get(init), |s, x| s * x))
    }
    pub fn cumsum_from(init: &LDouble, xs: &[f64], ys: &mut [f64]) -> LDouble {
        let mut s = get(init);
        for (x, y) in xs.iter().zip(ys.iter_mut()) {
            s += x;
            *y = s;
        }
        put(s)
    }
    pub fn sum_dev(m: &LDouble, xs: &[f64]) -> LDouble {
        let m = get(m);
        put(xs.iter().fold(0.0, |t, x| t + (x - m)))
    }
    pub fn cmp(a: &LDouble, b: &LDouble) -> i32 {
        match get(a).partial_cmp(&get(b)) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Equal) => 0,
            Some(std::cmp::Ordering::Greater) => 1,
            None => 2,
        }
    }
}

impl LDouble {
    pub const ZERO: LDouble = LDouble([0; 16]);
    /// 1.0: significand 0x8000000000000000, biased exponent 0x3fff.
    #[cfg(target_arch = "x86_64")]
    pub const ONE: LDouble = LDouble([0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0x3f, 0, 0, 0, 0, 0, 0]);
    #[cfg(not(target_arch = "x86_64"))]
    pub const ONE: LDouble = LDouble([0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0, 0, 0, 0]);

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
