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

/// x86-64 SSE binary-operation NaN semantics, as exhibited by the compiled
/// libgcc `__muldc3`/`__divdc3` (and any gcc-compiled C): a NaN operand is
/// returned quietized, the destination (left) operand's payload wins when
/// both are NaN, and an invalid operation on non-NaN operands yields the
/// QNaN indefinite `0xfff8000000000000`. LLVM differs in exactly the cases
/// that decide payload bits: it constant-folds invalid operations on
/// literals to the positive default NaN (e.g. `INFINITY * 0.0`) and may
/// commute operands, so which NaN payload survives `nan + nan` depends on
/// the codegen. The kernels therefore recompute through these helpers
/// whenever the fast result contains a NaN (see [`CArith`]).
pub(crate) mod sse {
    /// SSE quiets a propagated signalling NaN by setting the quiet bit.
    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    #[inline(always)]
    fn quietize(x: f64) -> f64 {
        f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
    }

    // The QNaN indefinite every invalid x86-64 SSE operation produces
    // (0*Inf, Inf-Inf, 0/0, sqrt(-1), ...); LLVM folds literals to
    // 0x7ff8000000000000 instead.
    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    const INDEFINITE: f64 = f64::from_bits(0xfff8_0000_0000_0000);

    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    #[cold]
    #[inline(never)]
    fn propagate(a: f64, b: f64) -> f64 {
        if a.is_nan() {
            quietize(a)
        } else if b.is_nan() {
            quietize(b)
        } else {
            INDEFINITE
        }
    }

    pub const ADD: u8 = 0;
    pub const SUB: u8 = 1;
    pub const MUL: u8 = 2;

    /// `d OP s` on two lanes at once, `d` the destination: inline assembly
    /// fixes the operand order, which shows as which NaN payload survives.
    #[inline(always)]
    pub fn packed<const OP: u8>(d: [f64; 2], s: [f64; 2]) -> [f64; 2] {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: [f64; 2] and __m128d are the same 16 bytes; SSE2 is part
        // of x86-64.
        unsafe {
            use core::arch::x86_64::__m128d;
            let mut d = core::mem::transmute::<[f64; 2], __m128d>(d);
            let s = core::mem::transmute::<[f64; 2], __m128d>(s);
            match OP {
                ADD => {
                    core::arch::asm!("addpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
                SUB => {
                    core::arch::asm!("subpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
                _ => {
                    core::arch::asm!("mulpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
            }
            core::mem::transmute::<__m128d, [f64; 2]>(d)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let op = match OP {
                ADD => add,
                SUB => sub,
                _ => mul,
            };
            [op(d[0], s[0]), op(d[1], s[1])]
        }
    }

    // On x86-64 the hardware instructions are these semantics: inline
    // assembly keeps LLVM from commuting or folding them, at the cost of
    // one instruction per operation.
    #[cfg(target_arch = "x86_64")]
    macro_rules! sse_op {
        ($name:ident, $insn:literal, $op:tt) => {
            #[inline(always)]
            pub fn $name(a: f64, b: f64) -> f64 {
                let mut r = a;
                // SAFETY: a scalar SSE2 operation on registers.
                unsafe {
                    core::arch::asm!(
                        concat!($insn, " {r}, {b}"),
                        r = inout(xmm_reg) r,
                        b = in(xmm_reg) b,
                        options(pure, nomem, nostack, preserves_flags),
                    );
                }
                r
            }
        };
    }
    #[cfg(not(target_arch = "x86_64"))]
    macro_rules! sse_op {
        ($name:ident, $insn:literal, $op:tt) => {
            #[inline(always)]
            pub fn $name(a: f64, b: f64) -> f64 {
                let r = a $op b;
                if r.is_nan() { propagate(a, b) } else { r }
            }
        };
    }
    sse_op!(add, "addsd", +);
    sse_op!(sub, "subsd", -);
    sse_op!(mul, "mulsd", *);
    sse_op!(div, "divsd", /);
}

/// Whether [`sse`] costs no more than plain operations (it is the hardware
/// instruction), so the kernels can run once with exact NaN semantics
/// instead of a fast pass and an exact recomputation.
const SSE_IS_NATIVE: bool = cfg!(target_arch = "x86_64");

/// The arithmetic the complex kernels are instantiated with. `Fast` is the
/// plain IEEE operations — bit-identical with the compiled C library for
/// every non-NaN result, whatever LLVM does with operand order and literal
/// folding (finite arithmetic is never reassociated). `Exact` adds x86-64
/// SSE NaN semantics (see [`sse`]) so NaN payload bits match the C library
/// too; it only ever runs for results the fast path reported as NaN.
trait CArith {
    fn add(a: f64, b: f64) -> f64;
    fn sub(a: f64, b: f64) -> f64;
    fn mul(a: f64, b: f64) -> f64;
    fn div(a: f64, b: f64) -> f64;
}

struct FastArith;
impl CArith for FastArith {
    #[inline(always)]
    fn add(a: f64, b: f64) -> f64 {
        a + b
    }
    #[inline(always)]
    fn sub(a: f64, b: f64) -> f64 {
        a - b
    }
    #[inline(always)]
    fn mul(a: f64, b: f64) -> f64 {
        a * b
    }
    #[inline(always)]
    fn div(a: f64, b: f64) -> f64 {
        a / b
    }
}

struct ExactArith;
impl CArith for ExactArith {
    #[inline(always)]
    fn add(a: f64, b: f64) -> f64 {
        sse::add(a, b)
    }
    #[inline(always)]
    fn sub(a: f64, b: f64) -> f64 {
        sse::sub(a, b)
    }
    #[inline(always)]
    fn mul(a: f64, b: f64) -> f64 {
        sse::mul(a, b)
    }
    #[inline(always)]
    fn div(a: f64, b: f64) -> f64 {
        sse::div(a, b)
    }
}

// R: libgcc2.c:__muldc3 (the C99 `x * y` R's complex_binary uses).
#[inline]
pub fn cmul(a: Complex, b: Complex) -> Complex {
    if SSE_IS_NATIVE {
        return cmul_impl::<ExactArith>(a, b);
    }
    let r = cmul_impl::<FastArith>(a, b);
    if r.re.is_nan() || r.im.is_nan() { cmul_impl::<ExactArith>(a, b) } else { r }
}

/// A packed SSE2 operation on both parts of two complex numbers, `a` the
/// destination operand: one instruction, with the operand order fixed (in
/// plain Rust LLVM may commute an addition, and which NaN payload survives
/// shows as NA vs NaN).
#[inline(always)]
fn sse_pair<const OP: u8>(a: Complex, b: Complex) -> Complex {
    let [re, im] = sse::packed::<OP>([a.re, a.im], [b.re, b.im]);
    Complex { re, im }
}

// R: complex.c:complex_binary PLUSOP. GCC commuted the adds of the
// compiled loop: the destination operand of each addsd holds the *second*
// operand's part, so with two NaN operands x2's payload wins (observable:
// `(0/0+1i)+(1+NA)` is NA, not NaN+NaNi). One `addpd` with `b` as the
// destination: a single pass, also for NaN sums (escaping Mandelbrot
// iterations are all NaN).
#[inline(always)]
pub fn cadd(a: Complex, b: Complex) -> Complex {
    sse_pair::<{ sse::ADD }>(b, a)
}

// R: complex.c:complex_binary MINUSOP. The subtractions are not
// commutative, so the compiled code has no freedom: x1 - x2 with the
// minuend as destination operand.
#[inline(always)]
pub fn csub(a: Complex, b: Complex) -> Complex {
    sse_pair::<{ sse::SUB }>(a, b)
}

/// `f` over two slices of the same length, unrolled by four and written
/// into the result's spare capacity: LLVM does not unroll loops around
/// inline assembly, and the loop overhead would otherwise cost more than
/// the operation.
#[inline(always)]
pub fn zip_unrolled(a: &[Complex], b: &[Complex], f: impl Fn(Complex, Complex) -> Complex) -> Vec<Complex> {
    let n = a.len().min(b.len());
    let mut out: Vec<Complex> = Vec::with_capacity(n);
    {
        let dst = &mut out.spare_capacity_mut()[..n];
        let (a, b) = (&a[..n], &b[..n]);
        let mut d4 = dst.chunks_exact_mut(4);
        let (mut a4, mut b4) = (a.chunks_exact(4), b.chunks_exact(4));
        for ((o, x), y) in (&mut d4).zip(&mut a4).zip(&mut b4) {
            o[0].write(f(x[0], y[0]));
            o[1].write(f(x[1], y[1]));
            o[2].write(f(x[2], y[2]));
            o[3].write(f(x[3], y[3]));
        }
        for ((o, &x), &y) in d4.into_remainder().iter_mut().zip(a4.remainder()).zip(b4.remainder()) {
            o.write(f(x, y));
        }
    }
    // SAFETY: the loops above wrote all n elements.
    unsafe { out.set_len(n) };
    out
}

#[inline(always)]
fn cmul_impl<T: CArith>(a: Complex, b: Complex) -> Complex {
    let (a, b, c, d) = (a.re, a.im, b.re, b.im);
    let ac = T::mul(a, c);
    let bd = T::mul(b, d);
    let ad = T::mul(a, d);
    // gcc 14's compiled __muldc3 computes `c * b` (the destination operand
    // of its mulsd is the one holding c), so with two NaN operands c's
    // payload wins: observable in the result bits.
    let bc = T::mul(c, b);
    let x = T::sub(ac, bd);
    let y = T::add(ad, bc);
    // The recovery below changes the result only if an operand or a
    // product is infinite; NaN operands (escaped iterations) skip it.
    let inf = |v: f64| v.abs() == f64::INFINITY;
    if x.is_nan() && y.is_nan() && (inf(a) | inf(b) | inf(c) | inf(d) | inf(ac) | inf(bd) | inf(ad) | inf(bc)) {
        return cmul_recover::<T>([a, b, c, d], [ac, bd, ad, bc], x, y);
    }
    Complex::new(x, y)
}

/// __muldc3's recovery of infinities that computed as NaN + iNaN.
#[cold]
#[inline(never)]
fn cmul_recover<T: CArith>([a, b, c, d]: [f64; 4], [ac, bd, ad, bc]: [f64; 4], mut x: f64, mut y: f64) -> Complex {
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
        x = T::mul(f64::INFINITY, T::sub(T::mul(a, c), T::mul(b, d)));
        y = T::mul(f64::INFINITY, T::add(T::mul(a, d), T::mul(b, c)));
    }
    Complex::new(x, y)
}

/// [`cmul`] on two lanes at once, `cmul(x[k], y[k])`, with the same
/// result bits: the arithmetic is the packed form of the same SSE2
/// instructions (each lane follows the scalar rules, so NaN payloads
/// survive as in `cmul`), and `__muldc3`'s recovery runs on both lanes
/// without branches, its result kept where a lane needs it.
#[cfg(target_arch = "x86_64")]
pub(crate) mod pair {
    use crate::value::Complex;
    use core::arch::x86_64 as x86;
    use core::arch::x86_64::__m128d as V;

    const ADD: u8 = 0;
    const SUB: u8 = 1;
    const MUL: u8 = 2;

    // Safe wrappers of the SSE2 intrinsics used here. SAFETY (for every
    // `unsafe` block of this module): SSE2 is part of the x86-64 baseline,
    // and these operate on registers only.

    /// `d OP s` on both lanes, `d` the destination operand.
    #[inline(always)]
    fn op<const OP: u8>(mut d: V, s: V) -> V {
        // SAFETY: see above.
        unsafe {
            match OP {
                ADD => {
                    core::arch::asm!("addpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
                SUB => {
                    core::arch::asm!("subpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
                _ => {
                    core::arch::asm!("mulpd {d}, {s}", d = inout(xmm_reg) d, s = in(xmm_reg) s, options(pure, nomem, nostack, preserves_flags))
                }
            }
        }
        d
    }

    #[inline(always)]
    fn and(a: V, b: V) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_and_pd(a, b) }
    }

    #[inline(always)]
    fn or(a: V, b: V) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_or_pd(a, b) }
    }

    /// `!a & b`
    #[inline(always)]
    fn andnot(a: V, b: V) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_andnot_pd(a, b) }
    }

    #[inline(always)]
    fn splat(x: f64) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_set1_pd(x) }
    }

    #[inline(always)]
    fn bits(x: u64) -> V {
        splat(f64::from_bits(x))
    }

    #[inline(always)]
    fn any(m: V) -> bool {
        // SAFETY: see above.
        unsafe { x86::_mm_movemask_pd(m) != 0 }
    }

    #[inline(always)]
    fn sel(m: V, a: V, b: V) -> V {
        or(and(m, a), andnot(m, b))
    }

    #[inline(always)]
    fn isnan(v: V) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_cmpunord_pd(v, v) }
    }

    #[inline(always)]
    fn isinf(v: V) -> V {
        // SAFETY: see above.
        unsafe { x86::_mm_cmpeq_pd(and(v, bits(0x7fff_ffff_ffff_ffff)), splat(f64::INFINITY)) }
    }

    /// `copysign(m ? 1 : 0, v)`.
    #[inline(always)]
    fn sign_one(m: V, v: V) -> V {
        or(and(m, splat(1.0)), and(v, bits(1 << 63)))
    }

    /// `v`, with the lanes in `m` that are NaN replaced by `copysign(0, v)`.
    #[inline(always)]
    fn nan_to_zero(m: V, v: V) -> V {
        sel(and(m, isnan(v)), and(v, bits(1 << 63)), v)
    }

    /// The real and imaginary parts of two complex numbers, as two lanes each.
    #[cfg(test)]
    #[inline(always)]
    pub fn split(x: [Complex; 2]) -> (V, V) {
        // SAFETY: see above.
        unsafe { (x86::_mm_set_pd(x[1].re, x[0].re), x86::_mm_set_pd(x[1].im, x[0].im)) }
    }

    #[inline(always)]
    pub fn join(re: V, im: V) -> [Complex; 2] {
        // SAFETY: see above.
        unsafe {
            let hi = |v: V| x86::_mm_cvtsd_f64(x86::_mm_unpackhi_pd(v, v));
            [Complex::new(x86::_mm_cvtsd_f64(re), x86::_mm_cvtsd_f64(im)), Complex::new(hi(re), hi(im))]
        }
    }

    /// `cmul` (see `cmul_impl` and `cmul_recover`) on parts as lanes.
    // R: libgcc2.c:__muldc3
    #[inline(always)]
    pub fn mul(a: V, b: V, c: V, d: V) -> (V, V) {
        let ac = op::<MUL>(a, c);
        let bd = op::<MUL>(b, d);
        let ad = op::<MUL>(a, d);
        let bc = op::<MUL>(c, b);
        let x = op::<SUB>(ac, bd);
        let y = op::<ADD>(ad, bc);
        let need = and(isnan(x), isnan(y));
        if !any(need) {
            return (x, y);
        }
        recover(need, [a, b, c, d], [ac, bd, ad, bc], x, y)
    }

    #[inline(always)]
    fn recover(need: V, [a, b, c, d]: [V; 4], [ac, bd, ad, bc]: [V; 4], x: V, y: V) -> (V, V) {
        // if (isinf (a) || isinf (b)): box the infinity, NaNs of the other
        // factor to 0.
        let (ia, ib) = (isinf(a), isinf(b));
        let s1 = or(ia, ib);
        let a = sel(s1, sign_one(ia, a), a);
        let b = sel(s1, sign_one(ib, b), b);
        let c = nan_to_zero(s1, c);
        let d = nan_to_zero(s1, d);
        // if (isinf (c) || isinf (d))
        let (ic, id) = (isinf(c), isinf(d));
        let s2 = or(ic, id);
        let c = sel(s2, sign_one(ic, c), c);
        let d = sel(s2, sign_one(id, d), d);
        let a = nan_to_zero(s2, a);
        let b = nan_to_zero(s2, b);
        // if (!recalc && (isinf (ac) || ...)): NaNs to 0.
        let recalc = or(s1, s2);
        let prod_inf = or(or(isinf(ac), isinf(bd)), or(isinf(ad), isinf(bc)));
        let s3 = andnot(recalc, prod_inf);
        let (a, b, c, d) = (nan_to_zero(s3, a), nan_to_zero(s3, b), nan_to_zero(s3, c), nan_to_zero(s3, d));
        let recalc = or(recalc, s3);
        let inf = splat(f64::INFINITY);
        let xr = op::<MUL>(inf, op::<SUB>(op::<MUL>(a, c), op::<MUL>(b, d)));
        let yr = op::<MUL>(inf, op::<ADD>(op::<MUL>(a, d), op::<MUL>(b, c)));
        let m = and(need, recalc);
        (sel(m, xr, x), sel(m, yr, y))
    }

    /// `cmul(x[k], y[k])` for two pairs.
    #[cfg(test)]
    pub fn cmul2(x: [Complex; 2], y: [Complex; 2]) -> [Complex; 2] {
        let (a, b) = split(x);
        let (c, d) = split(y);
        let (re, im) = mul(a, b, c, d);
        join(re, im)
    }

    /// `cpow(x[i], y)` for every element, `y` an integer power `k > 1`
    /// that `cpow` takes to `r_cpow_n`: its multiplications two elements at
    /// a time (the same for every element, as `k` is).
    pub fn cpow_n(x: &[Complex], y: Complex, k: i32) -> Vec<Complex> {
        debug_assert!(k > 1 && y.im == 0.0 && y.re == k as f64);
        let mut out: Vec<Complex> = Vec::with_capacity(x.len());
        let x2 = x.chunks_exact(2);
        let xr = x2.remainder();
        let one = (splat(1.0), splat(0.0));
        for p in x2 {
            // (`cpow` of zero is not `r_cpow_n`'s.)
            if p.iter().any(|c| c.re == 0.0 && c.im == 0.0) {
                out.extend(p.iter().map(|&c| super::cpow(c, y)));
                continue;
            }
            let (mut xa, mut xb) = load(p.try_into().unwrap());
            let mut z = one;
            let mut k = k;
            // R: complex.c:R_cpow_n
            while k > 0 {
                if k & 1 != 0 {
                    z = mul(z.0, z.1, xa, xb);
                }
                if k == 1 {
                    break;
                }
                k >>= 1;
                (xa, xb) = mul(xa, xb, xa, xb);
            }
            out.extend_from_slice(&join(z.0, z.1));
        }
        out.extend(xr.iter().map(|&c| super::cpow(c, y)));
        out
    }

    /// The parts of `p[0]` and `p[1]` as lanes.
    #[inline(always)]
    fn load(p: &[Complex; 2]) -> (V, V) {
        let f = p.as_ptr() as *const f64;
        // SAFETY: see above; `f` points to the four f64 of `p`.
        unsafe {
            let (lo, hi) = (x86::_mm_loadu_pd(f), x86::_mm_loadu_pd(f.add(2)));
            (x86::_mm_unpacklo_pd(lo, hi), x86::_mm_unpackhi_pd(lo, hi))
        }
    }

    /// `f2` (on parts as lanes) for each pair of elements of `x` and `y`
    /// (of the same length), `f` for a last odd one.
    #[inline(always)]
    pub fn zip(
        x: &[Complex],
        y: &[Complex],
        f2: impl Fn(V, V, V, V) -> (V, V),
        f: impl Fn(Complex, Complex) -> Complex,
    ) -> Vec<Complex> {
        let n = x.len().min(y.len());
        let mut out: Vec<Complex> = Vec::with_capacity(n);
        {
            let dst = &mut out.spare_capacity_mut()[..n];
            let (x2, y2) = (x[..n].chunks_exact(2), y[..n].chunks_exact(2));
            let (xr, yr) = (x2.remainder(), y2.remainder());
            let mut d2 = dst.chunks_exact_mut(2);
            for ((o, p), q) in (&mut d2).zip(x2).zip(y2) {
                let (a, b) = load(p.try_into().unwrap());
                let (c, d) = load(q.try_into().unwrap());
                let (re, im) = f2(a, b, c, d);
                let o = o.as_mut_ptr() as *mut f64;
                // SAFETY: see above; `o` points to two elements of spare
                // capacity.
                unsafe {
                    x86::_mm_storeu_pd(o, x86::_mm_unpacklo_pd(re, im));
                    x86::_mm_storeu_pd(o.add(2), x86::_mm_unpackhi_pd(re, im));
                }
            }
            for ((o, &p), &q) in d2.into_remainder().iter_mut().zip(xr).zip(yr) {
                o.write(f(p, q));
            }
        }
        // SAFETY: the loops above wrote all n elements.
        unsafe { out.set_len(n) };
        out
    }
}

/// C's `copysign` (Rust's has the same behavior).
fn copysign(x: f64, y: f64) -> f64 {
    f64::copysign(x, y)
}

// R: libgcc2.c:__divdc3 (the C99 `x / y` R's complex_binary uses).
pub fn cdiv(a: Complex, b: Complex) -> Complex {
    if SSE_IS_NATIVE {
        return cdiv_impl::<ExactArith>(a, b);
    }
    let r = cdiv_impl::<FastArith>(a, b);
    if r.re.is_nan() || r.im.is_nan() { cdiv_impl::<ExactArith>(a, b) } else { r }
}

fn cdiv_impl<T: CArith>(a: Complex, b: Complex) -> Complex {
    let (mut a, mut b, mut c, mut d) = (a.re, a.im, b.re, b.im);
    let (mut x, mut y);
    if c.abs() < d.abs() {
        // Prevent underflow when denominator is near max representable.
        if d.abs() >= RBIG {
            a = T::div(a, 2.0);
            b = T::div(b, 2.0);
            c = T::div(c, 2.0);
            d = T::div(d, 2.0);
        }
        // Avoid overflow/underflow issues when c and d are small.
        if d.abs() < RMIN2 {
            a = T::mul(a, RMINSCAL);
            b = T::mul(b, RMINSCAL);
            c = T::mul(c, RMINSCAL);
            d = T::mul(d, RMINSCAL);
        } else if (a.abs() < RMIN && b.abs() < RMAX2 && d.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && d.abs() < RMAX2)
        {
            a = T::mul(a, RMINSCAL);
            b = T::mul(b, RMINSCAL);
            c = T::mul(c, RMINSCAL);
            d = T::mul(d, RMINSCAL);
        }
        let ratio = T::div(c, d);
        let denom = T::add(T::mul(c, ratio), d);
        // Choose alternate order of computation if ratio is subnormal.
        if ratio.abs() > RMIN {
            x = T::div(T::add(T::mul(a, ratio), b), denom);
            y = T::div(T::sub(T::mul(b, ratio), a), denom);
        } else {
            // gcc 14's __divdc3 emits `(a/d) * c`, not the source's
            // `c * (a/d)`: with NaN operands the destination operand's
            // payload wins, so the order is observable in the bits.
            x = T::div(T::add(T::mul(T::div(a, d), c), b), denom);
            y = T::div(T::sub(T::mul(T::div(b, d), c), a), denom);
        }
    } else {
        if c.abs() >= RBIG {
            a = T::div(a, 2.0);
            b = T::div(b, 2.0);
            c = T::div(c, 2.0);
            d = T::div(d, 2.0);
        }
        if c.abs() < RMIN2 {
            a = T::mul(a, RMINSCAL);
            b = T::mul(b, RMINSCAL);
            c = T::mul(c, RMINSCAL);
            d = T::mul(d, RMINSCAL);
        } else if (a.abs() < RMIN && b.abs() < RMAX2 && c.abs() < RMAX2)
            || (b.abs() < RMIN && a.abs() < RMAX2 && c.abs() < RMAX2)
        {
            a = T::mul(a, RMINSCAL);
            b = T::mul(b, RMINSCAL);
            c = T::mul(c, RMINSCAL);
            d = T::mul(d, RMINSCAL);
        }
        let ratio = T::div(d, c);
        let denom = T::add(T::mul(d, ratio), c);
        if ratio.abs() > RMIN {
            x = T::div(T::add(T::mul(b, ratio), a), denom);
            y = T::div(T::sub(b, T::mul(a, ratio)), denom);
        } else {
            // gcc 14 emits `(b/c) * d`, not the source's `d * (b/c)`,
            // and adds with the product as destination: with NaN
            // operands the destination operand's payload wins, so the
            // order is observable in the bits.
            x = T::div(T::add(T::mul(T::div(b, c), d), a), denom);
            y = T::div(T::sub(b, T::mul(T::div(a, c), d)), denom);
        }
    }
    // Recover infinities and zeros that computed as NaN+iNaN; the only
    // cases are nonzero/zero, infinite/finite, and finite/infinite.
    if x.is_nan() && y.is_nan() {
        if c == 0.0 && d == 0.0 && (!a.is_nan() || !b.is_nan()) {
            x = T::mul(f64::copysign(f64::INFINITY, c), a);
            y = T::mul(f64::copysign(f64::INFINITY, c), b);
        } else if (a.is_infinite() || b.is_infinite()) && c.is_finite() && d.is_finite() {
            let a2 = copysign(if a.is_infinite() { 1.0 } else { 0.0 }, a);
            let b2 = copysign(if b.is_infinite() { 1.0 } else { 0.0 }, b);
            x = T::mul(f64::INFINITY, T::add(T::mul(a2, c), T::mul(b2, d)));
            y = T::mul(f64::INFINITY, T::sub(T::mul(b2, c), T::mul(a2, d)));
        } else if (c.is_infinite() || d.is_infinite()) && a.is_finite() && b.is_finite() {
            let c2 = copysign(if c.is_infinite() { 1.0 } else { 0.0 }, c);
            let d2 = copysign(if d.is_infinite() { 1.0 } else { 0.0 }, d);
            x = T::mul(0.0, T::add(T::mul(a, c2), T::mul(b, d2)));
            y = T::mul(0.0, T::sub(T::mul(b, c2), T::mul(a, d2)));
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
    if (-2147483648.0..2147483648.0).contains(&x) { x as i32 } else { i32::MIN }
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
        } else if (0.5..1.0).contains(&absx) && absy < f64::EPSILON / 2.0 && scale == 0 {
            let d2m1 = (absx - 1.0) * (absx + 1.0);
            result.re = d2m1.ln_1p() / 2.0;
        } else if (0.5..1.0).contains(&absx) && scale == 0 && absx * absx + absy * absy >= 0.5 {
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

// `(int)((DBL_MAX_EXP - 1) * M_LN2)` and its half, as the templates
// compute them (709 and 354).
const T_709: i32 = ((f64::MAX_EXP - 1) as f64 * std::f64::consts::LN_2) as i32;
const T_354: i32 = ((f64::MAX_EXP - 1) as f64 * std::f64::consts::LN_2 / 2.0) as i32;

// R: math/s_ccosh_template.c:__ccosh
pub fn ccosh(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut retval = Complex::new(0.0, 0.0);
    if rcls != FpClass::Nan && rcls != FpClass::Infinite {
        // Real part is finite.
        if icls != FpClass::Nan && icls != FpClass::Infinite {
            // Imaginary part is finite.
            let t = T_709;
            let (sinix, cosix);
            if x.im.abs() > f64::MIN_POSITIVE {
                sinix = x.im.sin();
                cosix = x.im.cos();
            } else {
                sinix = x.im;
                cosix = 1.0;
            }
            let (mut sinix, mut cosix) = (sinix, cosix);
            if x.re.abs() > t as f64 {
                let exp_t = (t as f64).exp();
                let mut rx = x.re.abs();
                if x.re.is_sign_negative() {
                    sinix = -sinix;
                }
                rx -= t as f64;
                sinix *= exp_t / 2.0;
                cosix *= exp_t / 2.0;
                if rx > t as f64 {
                    rx -= t as f64;
                    sinix *= exp_t;
                    cosix *= exp_t;
                }
                if rx > t as f64 {
                    // Overflow (original real part of x > 3t).
                    retval.re = f64::MAX * cosix;
                    retval.im = f64::MAX * sinix;
                } else {
                    let exp_val = rx.exp();
                    retval.re = exp_val * cosix;
                    retval.im = exp_val * sinix;
                }
            } else {
                retval.re = x.re.cosh() * cosix;
                retval.im = x.re.sinh() * sinix;
            }
        } else {
            retval.im = if x.re == 0.0 { 0.0 } else { f64::NAN };
            retval.re = x.im - x.im;
        }
    } else if rcls == FpClass::Infinite {
        // Real part is infinite.
        if icls != FpClass::Nan && icls != FpClass::Infinite && icls != FpClass::Zero {
            // Imaginary part is finite.
            let (sinix, cosix) = if x.im.abs() > f64::MIN_POSITIVE { (x.im.sin(), x.im.cos()) } else { (x.im, 1.0) };
            retval.re = f64::copysign(f64::INFINITY, cosix);
            retval.im = f64::copysign(f64::INFINITY, sinix) * f64::copysign(1.0, x.re);
        } else if icls == FpClass::Zero {
            // Imaginary part is 0.0.
            retval.re = f64::INFINITY;
            retval.im = x.im * f64::copysign(1.0, x.re);
        } else {
            retval.re = f64::INFINITY;
            retval.im = x.im - x.im;
        }
    } else {
        retval.re = f64::NAN;
        retval.im = if x.im == 0.0 { x.im } else { f64::NAN };
    }
    retval
}

// R: math/s_ccos_template.c:__ccos (ccosh of the parts swapped and the
// imaginary part negated).
pub fn ccos(x: Complex) -> Complex {
    ccosh(Complex::new(-x.im, x.re))
}

// R: math/s_csin_template.c:__csin
pub fn csin(x: Complex) -> Complex {
    let negate = x.re.is_sign_negative();
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    // The template negates the real part in place first.
    let xre = x.re.abs();
    let mut retval = Complex::new(0.0, 0.0);
    if icls != FpClass::Nan && icls != FpClass::Infinite {
        // Imaginary part is finite.
        if rcls != FpClass::Nan && rcls != FpClass::Infinite {
            // Real part is finite.
            let t = T_709;
            let (sinix, cosix);
            if xre > f64::MIN_POSITIVE {
                sinix = xre.sin();
                cosix = xre.cos();
            } else {
                sinix = xre;
                cosix = 1.0;
            }
            let (mut sinix, mut cosix) = (sinix, cosix);
            if negate {
                sinix = -sinix;
            }
            if x.im.abs() > t as f64 {
                let exp_t = (t as f64).exp();
                let mut ix = x.im.abs();
                if x.im.is_sign_negative() {
                    cosix = -cosix;
                }
                ix -= t as f64;
                sinix *= exp_t / 2.0;
                cosix *= exp_t / 2.0;
                if ix > t as f64 {
                    ix -= t as f64;
                    sinix *= exp_t;
                    cosix *= exp_t;
                }
                if ix > t as f64 {
                    // Overflow (original imaginary part of x > 3t).
                    retval.re = f64::MAX * sinix;
                    retval.im = f64::MAX * cosix;
                } else {
                    let exp_val = ix.exp();
                    retval.re = exp_val * sinix;
                    retval.im = exp_val * cosix;
                }
            } else {
                retval.re = x.im.cosh() * sinix;
                retval.im = x.im.sinh() * cosix;
            }
        } else if icls == FpClass::Zero {
            // Imaginary part is 0.0.
            retval.re = xre - xre;
            retval.im = x.im;
        } else {
            retval.re = f64::NAN;
            retval.im = f64::NAN;
        }
    } else if icls == FpClass::Infinite {
        // Imaginary part is infinite.
        if rcls == FpClass::Zero {
            // Real part is 0.0.
            retval.re = f64::copysign(0.0, if negate { -1.0 } else { 1.0 });
            retval.im = x.im;
        } else if rcls != FpClass::Nan && rcls != FpClass::Infinite {
            // Real part is finite.
            let (sinix, cosix) = if xre > f64::MIN_POSITIVE { (xre.sin(), xre.cos()) } else { (xre, 1.0) };
            retval.re = f64::copysign(f64::INFINITY, sinix);
            retval.im = f64::copysign(f64::INFINITY, cosix);
            if negate {
                retval.re = -retval.re;
            }
            if x.im.is_sign_negative() {
                retval.im = -retval.im;
            }
        } else {
            retval.re = xre - xre;
            retval.im = f64::INFINITY;
        }
    } else {
        if rcls == FpClass::Zero {
            retval.re = f64::copysign(0.0, if negate { -1.0 } else { 1.0 });
        } else {
            retval.re = f64::NAN;
        }
        retval.im = f64::NAN;
    }
    retval
}

// R: math/s_csinh_template.c:__csinh
pub fn csinh(x: Complex) -> Complex {
    let negate = x.re.is_sign_negative();
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let xre = x.re.abs();
    let mut retval = Complex::new(0.0, 0.0);
    if rcls != FpClass::Nan && rcls != FpClass::Infinite {
        // Real part is finite.
        if icls != FpClass::Nan && icls != FpClass::Infinite {
            // Imaginary part is finite.
            let t = T_709;
            let (mut sinix, mut cosix) =
                if x.im.abs() > f64::MIN_POSITIVE { (x.im.sin(), x.im.cos()) } else { (x.im, 1.0) };
            if negate {
                cosix = -cosix;
            }
            if xre > t as f64 {
                let exp_t = (t as f64).exp();
                let mut rx = xre;
                // The template tests the sign of the (already absolute)
                // real part: never taken.
                if xre.is_sign_negative() {
                    cosix = -cosix;
                }
                rx -= t as f64;
                sinix *= exp_t / 2.0;
                cosix *= exp_t / 2.0;
                if rx > t as f64 {
                    rx -= t as f64;
                    sinix *= exp_t;
                    cosix *= exp_t;
                }
                if rx > t as f64 {
                    // Overflow (original real part of x > 3t).
                    retval.re = f64::MAX * cosix;
                    retval.im = f64::MAX * sinix;
                } else {
                    let exp_val = rx.exp();
                    retval.re = exp_val * cosix;
                    retval.im = exp_val * sinix;
                }
            } else {
                retval.re = xre.sinh() * cosix;
                retval.im = xre.cosh() * sinix;
            }
        } else if rcls == FpClass::Zero {
            // Real part is 0.0.
            retval.re = f64::copysign(0.0, if negate { -1.0 } else { 1.0 });
            retval.im = x.im - x.im;
        } else {
            retval.re = f64::NAN;
            retval.im = f64::NAN;
        }
    } else if rcls == FpClass::Infinite {
        // Real part is infinite.
        if icls != FpClass::Nan && icls != FpClass::Infinite && icls != FpClass::Zero {
            // Imaginary part is finite.
            let (sinix, cosix) = if x.im.abs() > f64::MIN_POSITIVE { (x.im.sin(), x.im.cos()) } else { (x.im, 1.0) };
            retval.re = f64::copysign(f64::INFINITY, cosix);
            retval.im = f64::copysign(f64::INFINITY, sinix);
            if negate {
                retval.re = -retval.re;
            }
        } else if icls == FpClass::Zero {
            // Imaginary part is 0.0.
            retval.re = if negate { -f64::INFINITY } else { f64::INFINITY };
            retval.im = x.im;
        } else {
            retval.re = f64::INFINITY;
            retval.im = x.im - x.im;
        }
    } else {
        retval.re = f64::NAN;
        retval.im = if x.im == 0.0 { x.im } else { f64::NAN };
    }
    retval
}

// R: math/s_ctan_template.c:__ctan
pub fn ctan(x: Complex) -> Complex {
    let mut res = Complex::new(0.0, 0.0);
    if !x.re.is_finite() || !x.im.is_finite() {
        if x.im.is_infinite() {
            if x.re.is_finite() && x.re.abs() > 1.0 {
                let (sinrx, cosrx) = (x.re.sin(), x.re.cos());
                res.re = f64::copysign(0.0, sinrx * cosrx);
            } else {
                res.re = f64::copysign(0.0, x.re);
            }
            res.im = f64::copysign(1.0, x.im);
        } else if x.re == 0.0 {
            res = x;
        } else {
            res.re = f64::NAN;
            res.im = if x.im == 0.0 { x.im } else { f64::NAN };
        }
    } else {
        let t = T_354;
        // tan(x+iy) = (sin(2x) + i*sinh(2y))/(cos(2x) + cosh(2y))
        //   = (sin(x)*cos(x) + i*sinh(y)*cosh(y)/(cos(x)^2 + sinh(y)^2).
        let (sinrx, cosrx) = if x.re.abs() > f64::MIN_POSITIVE { (x.re.sin(), x.re.cos()) } else { (x.re, 1.0) };
        if x.im.abs() > t as f64 {
            // Avoid intermediate overflow when the real part of the
            // result may be subnormal.  Ignoring negligible terms, the
            // imaginary part is +/- 1, the real part is
            // sin(x)*cos(x)/sinh(y)^2 = 4*sin(x)*cos(x)/exp(2y).
            let exp_2t = (2.0 * t as f64).exp();
            res.im = f64::copysign(1.0, x.im);
            res.re = 4.0 * sinrx * cosrx;
            let mut im = x.im.abs();
            im -= t as f64;
            res.re /= exp_2t;
            if im > t as f64 {
                // Underflow (original imaginary part of x has absolute
                // value > 2t).
                res.re /= exp_2t;
            } else {
                res.re /= (2.0 * im).exp();
            }
        } else {
            let (sinhix, coshix) =
                if x.im.abs() > f64::MIN_POSITIVE { (x.im.sinh(), x.im.cosh()) } else { (x.im, 1.0) };
            let den =
                if sinhix.abs() > cosrx.abs() * f64::EPSILON { cosrx * cosrx + sinhix * sinhix } else { cosrx * cosrx };
            res.re = sinrx * cosrx / den;
            res.im = sinhix * coshix / den;
        }
    }
    res
}

// R: math/s_ctanh_template.c:__ctanh
pub fn ctanh(x: Complex) -> Complex {
    let mut res = Complex::new(0.0, 0.0);
    if !x.re.is_finite() || !x.im.is_finite() {
        if x.re.is_infinite() {
            res.re = f64::copysign(1.0, x.re);
            if x.im.is_finite() && x.im.abs() > 1.0 {
                let (sinix, cosix) = (x.im.sin(), x.im.cos());
                res.im = f64::copysign(0.0, sinix * cosix);
            } else {
                res.im = f64::copysign(0.0, x.im);
            }
        } else if x.im == 0.0 {
            res = x;
        } else {
            res.re = if x.re == 0.0 { x.re } else { f64::NAN };
            res.im = f64::NAN;
        }
    } else {
        let t = T_354;
        // tanh(x+iy) = (sinh(2x) + i*sin(2y))/(cosh(2x) + cos(2y))
        //   = (sinh(x)*cosh(x) + i*sin(y)*cos(y))/(sinh(x)^2 + cos(y)^2).
        let (sinix, cosix) = if x.im.abs() > f64::MIN_POSITIVE { (x.im.sin(), x.im.cos()) } else { (x.im, 1.0) };
        if x.re.abs() > t as f64 {
            // Avoid intermediate overflow when the imaginary part of
            // the result may be subnormal.  Ignoring negligible terms,
            // the real part is +/- 1, the imaginary part is
            // sin(y)*cos(y)/sinh(x)^2 = 4*sin(y)*cos(y)/exp(2x).
            let exp_2t = (2.0 * t as f64).exp();
            res.re = f64::copysign(1.0, x.re);
            res.im = 4.0 * sinix * cosix;
            let mut re = x.re.abs();
            re -= t as f64;
            res.im /= exp_2t;
            if re > t as f64 {
                // Underflow (original real part of x has absolute
                // value > 2t).
                res.im /= exp_2t;
            } else {
                res.im /= (2.0 * re).exp();
            }
        } else {
            let (sinhrx, coshrx) =
                if x.re.abs() > f64::MIN_POSITIVE { (x.re.sinh(), x.re.cosh()) } else { (x.re, 1.0) };
            let den =
                if sinhrx.abs() > cosix.abs() * f64::EPSILON { sinhrx * sinhrx + cosix * cosix } else { cosix * cosix };
            res.re = sinhrx * coshrx / den;
            res.im = sinix * cosix / den;
        }
    }
    res
}

// R: math/k_casinh_template.c:__kernel_casinh.  `adj` adjusts the
// imaginary part of the result for use in computing other functions.
fn kernel_casinh(x: Complex, adj: bool) -> Complex {
    let mut res = Complex::new(0.0, 0.0);
    // Avoid cancellation by reducing to the first quadrant.
    let rx = x.re.abs();
    let ix = x.im.abs();
    let y: Complex;
    if rx >= 1.0 / f64::EPSILON || ix >= 1.0 / f64::EPSILON {
        // For large x in the first quadrant, x + csqrt (1 + x * x)
        // is sufficiently close to 2 * x to make no significant
        // difference to the result; avoid possible overflow from
        // the squaring and addition.
        let mut yv = Complex::new(rx, ix);
        if adj {
            let t = yv.re;
            yv.re = f64::copysign(yv.im, x.im);
            yv.im = t;
        }
        y = yv;
        res = clog(y);
        res.re += std::f64::consts::LN_2;
    } else if rx >= 0.5 && ix < f64::EPSILON / 8.0 {
        let s = 1.0f64.hypot(rx);
        res.re = (rx + s).ln();
        res.im = if adj { f64::atan2(s, x.im) } else { f64::atan2(ix, s) };
    } else if rx < f64::EPSILON / 8.0 && ix >= 1.5 {
        let s = ((ix + 1.0) * (ix - 1.0)).sqrt();
        res.re = (ix + s).ln();
        res.im = if adj { f64::atan2(rx, f64::copysign(s, x.im)) } else { f64::atan2(s, rx) };
    } else if ix > 1.0 && ix < 1.5 && rx < 0.5 {
        if rx < f64::EPSILON * f64::EPSILON {
            let ix2m1 = (ix + 1.0) * (ix - 1.0);
            let s = ix2m1.sqrt();
            res.re = (2.0 * (ix2m1 + ix * s)).ln_1p() / 2.0;
            res.im = if adj { f64::atan2(rx, f64::copysign(s, x.im)) } else { f64::atan2(s, rx) };
        } else {
            let ix2m1 = (ix + 1.0) * (ix - 1.0);
            let rx2 = rx * rx;
            let f = rx2 * (2.0 + rx2 + 2.0 * ix * ix);
            let d = (ix2m1 * ix2m1 + f).sqrt();
            let dp = d + ix2m1;
            let dm = f / dp;
            let r1 = ((dm + rx2) / 2.0).sqrt();
            let r2 = rx * ix / r1;
            res.re = (rx2 + dp + 2.0 * (rx * r1 + ix * r2)).ln_1p() / 2.0;
            res.im = if adj { f64::atan2(rx + r1, f64::copysign(ix + r2, x.im)) } else { f64::atan2(ix + r2, rx + r1) };
        }
    } else if ix == 1.0 && rx < 0.5 {
        if rx < f64::EPSILON / 8.0 {
            res.re = (2.0 * (rx + rx.sqrt())).ln_1p() / 2.0;
            res.im = if adj { f64::atan2(rx.sqrt(), f64::copysign(1.0, x.im)) } else { f64::atan2(1.0, rx.sqrt()) };
        } else {
            let d = rx * (4.0 + rx * rx).sqrt();
            let s1 = ((d + rx * rx) / 2.0).sqrt();
            let s2 = ((d - rx * rx) / 2.0).sqrt();
            res.re = (rx * rx + d + 2.0 * (rx * s1 + s2)).ln_1p() / 2.0;
            res.im =
                if adj { f64::atan2(rx + s1, f64::copysign(1.0 + s2, x.im)) } else { f64::atan2(1.0 + s2, rx + s1) };
        }
    } else if ix < 1.0 && rx < 0.5 {
        if ix >= f64::EPSILON {
            if rx < f64::EPSILON * f64::EPSILON {
                let onemix2 = (1.0 + ix) * (1.0 - ix);
                let s = onemix2.sqrt();
                res.re = (2.0 * rx / s).ln_1p() / 2.0;
                res.im = if adj { f64::atan2(s, x.im) } else { f64::atan2(ix, s) };
            } else {
                let onemix2 = (1.0 + ix) * (1.0 - ix);
                let rx2 = rx * rx;
                let f = rx2 * (2.0 + rx2 + 2.0 * ix * ix);
                let d = (onemix2 * onemix2 + f).sqrt();
                let dp = d + onemix2;
                let dm = f / dp;
                let r1 = ((dp + rx2) / 2.0).sqrt();
                let r2 = rx * ix / r1;
                res.re = (rx2 + dm + 2.0 * (rx * r1 + ix * r2)).ln_1p() / 2.0;
                res.im =
                    if adj { f64::atan2(rx + r1, f64::copysign(ix + r2, x.im)) } else { f64::atan2(ix + r2, rx + r1) };
            }
        } else {
            let s = 1.0f64.hypot(rx);
            res.re = (2.0 * rx * (rx + s)).ln_1p() / 2.0;
            res.im = if adj { f64::atan2(s, x.im) } else { f64::atan2(ix, s) };
        }
    } else {
        let mut yv = Complex::new((rx - ix) * (rx + ix) + 1.0, 2.0 * rx * ix);
        yv = csqrt(yv);
        yv.re += rx;
        yv.im += ix;
        if adj {
            let t = yv.re;
            yv.re = f64::copysign(yv.im, x.im);
            yv.im = t;
        }
        y = yv;
        res = clog(y);
    }
    // Give results the correct sign for the original argument.
    res.re = f64::copysign(res.re, x.re);
    res.im = f64::copysign(res.im, if adj { 1.0 } else { x.im });
    res
}

// R: math/s_casinh_template.c:__casinh
pub fn casinh(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut res = Complex::new(0.0, 0.0);
    if rcls == FpClass::Nan || rcls == FpClass::Infinite || icls == FpClass::Nan || icls == FpClass::Infinite {
        if icls == FpClass::Infinite {
            res.re = f64::copysign(f64::INFINITY, x.re);
            if rcls == FpClass::Nan {
                res.im = f64::NAN;
            } else {
                res.im = f64::copysign(
                    if rcls != FpClass::Nan && rcls != FpClass::Infinite {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        std::f64::consts::FRAC_PI_4
                    },
                    x.im,
                );
            }
        } else if rcls == FpClass::Nan || rcls == FpClass::Infinite {
            res.re = x.re;
            if (rcls == FpClass::Infinite && icls != FpClass::Nan && icls != FpClass::Infinite)
                || (rcls == FpClass::Nan && icls == FpClass::Zero)
            {
                res.im = f64::copysign(0.0, x.im);
            } else {
                res.im = f64::NAN;
            }
        } else {
            res.re = f64::NAN;
            res.im = f64::NAN;
        }
    } else if rcls == FpClass::Zero && icls == FpClass::Zero {
        res = x;
    } else {
        res = kernel_casinh(x, false);
    }
    res
}

// R: math/s_casin_template.c:__casin
pub fn casin(x: Complex) -> Complex {
    let mut res = Complex::new(0.0, 0.0);
    if x.re.is_nan() || x.im.is_nan() {
        if x.re == 0.0 {
            res = x;
        } else if x.re.is_infinite() || x.im.is_infinite() {
            res.re = f64::NAN;
            res.im = f64::copysign(f64::INFINITY, x.im);
        } else {
            res.re = f64::NAN;
            res.im = f64::NAN;
        }
    } else {
        let y = casinh(Complex::new(-x.im, x.re));
        res.re = y.im;
        res.im = -y.re;
    }
    res
}

// R: math/s_cacos_template.c:__cacos
pub fn cacos(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut res = Complex::new(0.0, 0.0);
    if rcls == FpClass::Nan
        || rcls == FpClass::Infinite
        || icls == FpClass::Nan
        || icls == FpClass::Infinite
        || (rcls == FpClass::Zero && icls == FpClass::Zero)
    {
        let y = casin(x);
        res.re = std::f64::consts::FRAC_PI_2 - y.re;
        if res.re == 0.0 {
            res.re = 0.0;
        }
        res.im = -y.im;
    } else {
        let y = kernel_casinh(Complex::new(-x.im, x.re), true);
        res.re = y.im;
        res.im = y.re;
    }
    res
}

// R: math/s_catan_template.c:__catan
pub fn catan(x: Complex) -> Complex {
    let rcls = f64_classify(x.re);
    let icls = f64_classify(x.im);
    let mut res = Complex::new(0.0, 0.0);
    if rcls == FpClass::Nan || rcls == FpClass::Infinite || icls == FpClass::Nan || icls == FpClass::Infinite {
        if rcls == FpClass::Infinite {
            res.re = f64::copysign(std::f64::consts::FRAC_PI_2, x.re);
            res.im = f64::copysign(0.0, x.im);
        } else if icls == FpClass::Infinite {
            if rcls != FpClass::Nan && rcls != FpClass::Infinite {
                res.re = f64::copysign(std::f64::consts::FRAC_PI_2, x.re);
            } else {
                res.re = f64::NAN;
            }
            res.im = f64::copysign(0.0, x.im);
        } else if icls == FpClass::Zero {
            res.re = f64::NAN;
            res.im = f64::copysign(0.0, x.im);
        } else {
            res.re = f64::NAN;
            res.im = f64::NAN;
        }
    } else if rcls == FpClass::Zero && icls == FpClass::Zero {
        res = x;
    } else if x.re.abs() >= 16.0 / f64::EPSILON || x.im.abs() >= 16.0 / f64::EPSILON {
        res.re = f64::copysign(std::f64::consts::FRAC_PI_2, x.re);
        if x.re.abs() <= 1.0 {
            res.im = 1.0 / x.im;
        } else if x.im.abs() <= 1.0 {
            res.im = x.im / x.re / x.re;
        } else {
            let h = (x.re / 2.0).hypot(x.im / 2.0);
            res.im = x.im / h / h / 4.0;
        }
    } else {
        let mut den;
        let mut absx = x.re.abs();
        let mut absy = x.im.abs();
        if absx < absy {
            std::mem::swap(&mut absx, &mut absy);
        }
        if absy < f64::EPSILON / 2.0 {
            den = (1.0 - absx) * (1.0 + absx);
            if den == 0.0 {
                den = 0.0;
            }
        } else if absx >= 1.0 {
            den = (1.0 - absx) * (1.0 + absx) - absy * absy;
        } else if absx >= 0.75 || absy >= 0.5 {
            den = -x2y2m1(absx, absy);
        } else {
            den = (1.0 - absx) * (1.0 + absx) - absy * absy;
        }
        res.re = 0.5 * f64::atan2(2.0 * x.re, den);
        if x.im.abs() == 1.0 && x.re.abs() < f64::EPSILON * f64::EPSILON {
            res.im = f64::copysign(0.5, x.im) * (std::f64::consts::LN_2 - x.re.abs().ln());
        } else {
            let mut r2 = 0.0f64;
            if x.re.abs() >= f64::EPSILON * f64::EPSILON {
                r2 = x.re * x.re;
            }
            let mut num = x.im + 1.0;
            num = r2 + num * num;
            den = x.im - 1.0;
            den = r2 + den * den;
            let f = num / den;
            if f < 0.5 {
                res.im = 0.25 * f.ln();
            } else {
                num = 4.0 * x.im;
                res.im = 0.25 * (num / den).ln_1p();
            }
        }
    }
    res
}

// R: complex.c:z_tan — glibc's ctan with the imaginary part forced to
// +/-1 where some systems leave NaN.
pub fn z_tan(x: Complex) -> Complex {
    let y = x.im;
    let mut r = ctan(x);
    if y.is_finite() && y.abs() > 25.0 {
        r.im = if y < 0.0 { -1.0 } else { 1.0 };
    }
    r
}

// R: complex.c:z_asin — R's branch-cut formula, glibc's casin elsewhere.
// The result is the compiled expression `asin(t1 - t2) + ri*I`: a C99
// complex multiply and add, not a constructor (ri*0 puts ri's NaN into
// the real part too).
pub fn z_asin(x: Complex) -> Complex {
    if x.im == 0.0 && x.re.abs() > 1.0 {
        let x = x.re;
        let t1 = 0.5 * (x + 1.0).abs();
        let t2 = 0.5 * (x - 1.0).abs();
        let alpha = t1 + t2;
        let mut ri = (alpha + (alpha * alpha - 1.0).sqrt()).ln();
        // This comes from
        // 'z_asin() is continuous from below if x >= 1
        //   and continuous from above if x <= -1.'
        if x > 1.0 {
            ri *= -1.0;
        }
        cadd(Complex::new((t1 - t2).asin(), 0.0), cmul(Complex::new(ri, 0.0), Complex::new(0.0, 1.0)))
    } else {
        casin(x)
    }
}

// R: complex.c:z_acos — R's branch-cut formula, glibc's cacos elsewhere.
pub fn z_acos(x: Complex) -> Complex {
    if x.im == 0.0 && x.re.abs() > 1.0 {
        csub(Complex::new(std::f64::consts::FRAC_PI_2, 0.0), z_asin(x))
    } else {
        cacos(x)
    }
}

// R: complex.c:z_atan — R's branch-cut formula, glibc's catan elsewhere.
// `rr + ri*I` is a C99 complex multiply and add (see z_asin).
pub fn z_atan(x: Complex) -> Complex {
    if x.re == 0.0 && x.im.abs() > 1.0 {
        let y = x.im;
        let rr = if y > 0.0 { std::f64::consts::FRAC_PI_2 } else { -std::f64::consts::FRAC_PI_2 };
        let ri = 0.25 * (((y + 1.0) * (y + 1.0)) / ((y - 1.0) * (y - 1.0))).ln();
        cadd(Complex::new(rr, 0.0), cmul(Complex::new(ri, 0.0), Complex::new(0.0, 1.0)))
    } else {
        catan(x)
    }
}

// R: complex.c:z_acosh/z_asinh/z_atanh — the products are the compiled
// `z_acos(z) * I` etc. (`-I * z_asin(z * I)`), i.e. C99 complex
// multiplication (libgcc's __muldc3, robigo's cmul).
pub fn z_acosh(x: Complex) -> Complex {
    cmul(z_acos(x), Complex::new(0.0, 1.0))
}

pub fn z_asinh(x: Complex) -> Complex {
    cmul(Complex::new(-0.0, -1.0), z_asin(cmul(x, Complex::new(0.0, 1.0))))
}

pub fn z_atanh(x: Complex) -> Complex {
    cmul(Complex::new(-0.0, -1.0), z_atan(cmul(x, Complex::new(0.0, 1.0))))
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

    /// The two-lane multiply gives `cmul`'s bits for every pair of special
    /// values, in either lane and next to any other pair.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pair_cmul_matches_cmul() {
        let parts = [
            0.0,
            -0.0,
            1.0,
            -2.5,
            1e-310,
            1e308,
            -1e308,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::from_bits(0xfff8000000000000),
            f64::from_bits(0x7ff00000000007a2), // R's NA (signalling)
            f64::from_bits(0x7ff8000000000123),
        ];
        let mut vals = Vec::new();
        for &re in &parts {
            for &im in &parts {
                vals.push(z(re, im));
            }
        }
        let n = vals.len();
        for i in 0..n {
            for j in 0..n {
                // (Lane partners chosen to mix recovering and plain lanes.)
                let (x2, y2) = (vals[(i * 7 + j) % n], vals[(j * 5 + 3) % n]);
                let got = super::pair::cmul2([vals[i], x2], [vals[j], y2]);
                assert_eq!(bits(got[0]), bits(cmul(vals[i], vals[j])), "{:?} * {:?}", vals[i], vals[j]);
                assert_eq!(bits(got[1]), bits(cmul(x2, y2)), "{:?} * {:?}", x2, y2);
            }
        }
    }

    /// The two-lane integer power gives `cpow`'s bits.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn pair_cpow_n_matches_cpow() {
        let parts = [
            0.0,
            -0.0,
            1.0,
            -2.5,
            0.5,
            1e-200,
            1e200,
            1e308,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::from_bits(0x7ff00000000007a2),
        ];
        let mut vals = Vec::new();
        for &re in &parts {
            for &im in &parts {
                vals.push(z(re, im));
            }
        }
        vals.push(z(1.0, 2.0));
        for k in [2, 3, 4, 5, 7, 8, 13, 64, 65536] {
            let y = z(k as f64, 0.0);
            let got = super::pair::cpow_n(&vals, y, k);
            for (g, &v) in got.iter().zip(&vals) {
                assert_eq!(bits(*g), bits(cpow(v, y)), "{v:?} ^ {k}");
            }
        }
    }

    /// The unrolled loop gives the element-by-element results for every
    /// length (remainders included), NaN payloads and all.
    #[test]
    fn zip_unrolled_matches_elementwise() {
        let vals = [
            z(1.0, -2.0),
            z(f64::from_bits(0x7ff00000000007a2), 1.0),
            z(f64::from_bits(0xfff8000000000000), 0.0),
            z(f64::from_bits(0x7ff8000000000123), f64::INFINITY),
            z(-0.0, 3.5),
        ];
        for n in 0..12 {
            let a: Vec<Complex> = (0..n).map(|k| vals[k % 5]).collect();
            let b: Vec<Complex> = (0..n).map(|k| vals[(k * 3 + 1) % 5]).collect();
            for f in [cadd as fn(Complex, Complex) -> Complex, csub] {
                let got: Vec<(u64, u64)> = zip_unrolled(&a, &b, f).iter().map(|c| bits(*c)).collect();
                let want: Vec<(u64, u64)> = a.iter().zip(&b).map(|(&x, &y)| bits(f(x, y))).collect();
                assert_eq!(got, want, "n = {n}");
            }
        }
    }

    /// R's complex `+ - * / ^` and `%*%` are bit-identical with GNU R
    /// 4.5.0 itself (Debian r-base-core 4.5.0-3, the CI oracle), whose
    /// compiled code picks operand orders that decide NaN payloads (NA vs
    /// NaN): every pair of complex numbers whose parts are NA (signalling
    /// and quiet), NaNs with several payloads, +-Inf, +-0, finite, huge and
    /// subnormal values, recorded by `scripts/oracle-complex-arith.R`
    /// (230,496 results).
    #[test]
    fn gnu_r_oracle() {
        use crate::builtins::arith::{ArithOp, complex_binary};
        use std::io::Read;
        let raw = include_bytes!("complex_r_oracle.txt.gz");
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s).unwrap();
        let hex = |h: &str| f64::from_bits(u64::from_str_radix(h, 16).unwrap());
        let mut checked = 0usize;
        for line in s.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            let (a, b) = (z(hex(t[1]), hex(t[2])), z(hex(t[3]), hex(t[4])));
            let got = match t[0] {
                "matprod" => crate::builtins::array::cprod(0, &[a], 1, 1, &[b], 1, 1)[0],
                op => {
                    let op = match op {
                        "add" => ArithOp::Plus,
                        "sub" => ArithOp::Minus,
                        "mul" => ArithOp::Times,
                        "div" => ArithOp::Div,
                        "pow" => ArithOp::Pow,
                        other => panic!("bad op {other}"),
                    };
                    complex_binary(op, &[a], &[b], 1).ok().unwrap()[0]
                }
            };
            assert_eq!((got.re.to_bits(), got.im.to_bits()), (hex(t[5]).to_bits(), hex(t[6]).to_bits()), "{line}");
            checked += 1;
        }
        assert!(checked > 200_000, "{checked}");
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

    /// The trigonometric and hyperbolic complex kernels — glibc's
    /// ccos/csin/ctan/ccosh/csinh/ctanh/casin/cacos/catan and R's z_*
    /// wrappers (complex.c) — are bit-identical with the C library of
    /// the CI image (Debian 13, glibc 2.41): special values, the
    /// templates' branch-cut and reduction thresholds, and random bit
    /// patterns, recorded by `scripts/oracle-complex-trig.c` in the
    /// robigo-oracle:4.5.0 container (384,640 results).  Like
    /// `c_library_oracle` above, last bits can differ on other glibcs.
    #[test]
    fn c_library_trig_oracle() {
        use std::io::Read;
        let mut tally = std::collections::BTreeMap::<(String, i64), usize>::new();
        let mut bad3 = 0usize;
        let raw = include_bytes!("complex_trig_oracle.txt.gz");
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
            let a = z(hex(t[1]), hex(t[2]));
            let want = (hex(t[5]), hex(t[6]));
            let got = match t[0] {
                "ccos" => ccos(a),
                "csin" => csin(a),
                "ctan" => ctan(a),
                "ccosh" => ccosh(a),
                "csinh" => csinh(a),
                "ctanh" => ctanh(a),
                "casin" => casin(a),
                "cacos" => cacos(a),
                "catan" => catan(a),
                "ztan" => z_tan(a),
                "zasin" => z_asin(a),
                "zacos" => z_acos(a),
                "zatan" => z_atan(a),
                "zacosh" => z_acosh(a),
                "zasinh" => z_asinh(a),
                "zatanh" => z_atanh(a),
                other => panic!("bad op {other}"),
            };
            let (g, w) = ((got.re.to_bits(), got.im.to_bits()), (want.0.to_bits(), want.1.to_bits()));
            if std::env::var("ROBIGO_TALLY").is_ok() {
                if g != w {
                    let ulps = |a: u64, b: u64| (a as i64).wrapping_sub(b as i64).abs();
                    let d = ulps(g.0, w.0).max(ulps(g.1, w.1));
                    *tally.entry((t[0].to_string(), d)).or_insert(0usize) += 1;
                    if bad3 == 0 && d > 2 {
                        bad3 = 1;
                        println!("large diff: {line}");
                    }
                }
                checked += 1;
                continue;
            }
            assert_eq!(g, w, "{line}");
            checked += 1;
        }
        if std::env::var("ROBIGO_TALLY").is_ok() {
            let mut v: Vec<_> = tally.into_iter().collect();
            v.sort();
            println!("{checked} checked; mismatches (op, max ulp diff) x count: {v:?}");
            return;
        }
        assert!(checked == n && n > 300000, "{checked} of {n}");
    }
}
