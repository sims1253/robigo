//! `+` and `*` on doubles with GNU R's NaN payloads (see [`exact`] and
//! [`x_is_dest`]), used by `arith.rs`. In their own module (codegen unit):
//! added to arith.rs, they changed how LLVM compiled the VM's loop
//! (vm::run_with, +0.5..0.9% on five benchmarks) without being called
//! from it.

/// `+` and `*` on doubles with the operand order of GNU R's compiled
/// loops. When both operands are NaN, the result is the destination
/// operand's payload (quieted): x86-64 SSE keeps the destination's, and
/// which operand gcc made the destination shows as NA vs NaN. In plain
/// Rust LLVM may commute `+`/`*`, so these are inline assembly; packed
/// (`addpd`/`mulpd`, two elements per instruction) and unrolled by four,
/// as LLVM does not unroll loops around inline assembly. `-` and `/`
/// cannot be commuted and stay plain Rust.
pub(super) mod exact {
    use super::super::complex::sse;

    /// `d op s` on two lanes at once, `d` the destination.
    #[inline(always)]
    fn pair(mul: bool, d: [f64; 2], s: [f64; 2]) -> [f64; 2] {
        if mul { sse::packed::<{ sse::MUL }>(d, s) } else { sse::packed::<{ sse::ADD }>(d, s) }
    }

    /// `d op s`, `d` the destination.
    #[inline(always)]
    pub fn one(mul: bool, d: f64, s: f64) -> f64 {
        if mul { super::super::complex::sse::mul(d, s) } else { super::super::complex::sse::add(d, s) }
    }

    /// `out ++= [x[i] op y[i]]`, `x` the destination when `XD`.
    #[inline(always)]
    pub fn append_vv<const MUL: bool, const XD: bool>(out: &mut Vec<f64>, x: &[f64], y: &[f64]) {
        let n = x.len().min(y.len());
        let (x, y) = (&x[..n], &y[..n]);
        out.reserve(n);
        let start = out.len();
        {
            let dst = &mut out.spare_capacity_mut()[..n];
            let mut d8 = dst.chunks_exact_mut(8);
            let (mut x8, mut y8) = (x.chunks_exact(8), y.chunks_exact(8));
            for ((o, a), b) in (&mut d8).zip(&mut x8).zip(&mut y8) {
                for k in 0..4 {
                    let (p, q) = ([a[2 * k], a[2 * k + 1]], [b[2 * k], b[2 * k + 1]]);
                    let r = if XD { pair(MUL, p, q) } else { pair(MUL, q, p) };
                    o[2 * k].write(r[0]);
                    o[2 * k + 1].write(r[1]);
                }
            }
            for ((o, &a), &b) in d8.into_remainder().iter_mut().zip(x8.remainder()).zip(y8.remainder()) {
                o.write(if XD { one(MUL, a, b) } else { one(MUL, b, a) });
            }
        }
        // SAFETY: the loops above wrote the n elements after `start`.
        unsafe { out.set_len(start + n) };
    }

    /// `out ++= [v[i] op k]` (`+`/`*` commute, so only the destination
    /// matters): `v` is the destination when `VD`, else `k`.
    #[inline(always)]
    pub fn append_vk<const MUL: bool, const VD: bool>(out: &mut Vec<f64>, v: &[f64], k: f64) {
        let n = v.len();
        out.reserve(n);
        let start = out.len();
        {
            let dst = &mut out.spare_capacity_mut()[..n];
            let kk = [k, k];
            let mut d8 = dst.chunks_exact_mut(8);
            let mut v8 = v.chunks_exact(8);
            for (o, a) in (&mut d8).zip(&mut v8) {
                for j in 0..4 {
                    let p = [a[2 * j], a[2 * j + 1]];
                    let r = if VD { pair(MUL, p, kk) } else { pair(MUL, kk, p) };
                    o[2 * j].write(r[0]);
                    o[2 * j + 1].write(r[1]);
                }
            }
            for (o, &a) in d8.into_remainder().iter_mut().zip(v8.remainder()) {
                o.write(if VD { one(MUL, a, k) } else { one(MUL, k, a) });
            }
        }
        // SAFETY: the loops above wrote the n elements after `start`.
        unsafe { out.set_len(start + n) };
    }

    /// `own[i] = own[i] op t[i]`, `own` the destination when `OD`.
    #[inline(always)]
    pub fn in_place_v<const MUL: bool, const OD: bool>(own: &mut [f64], t: &[f64]) {
        let n = own.len().min(t.len());
        let (own, t) = (&mut own[..n], &t[..n]);
        let mut o8 = own.chunks_exact_mut(8);
        let mut t8 = t.chunks_exact(8);
        for (o, b) in (&mut o8).zip(&mut t8) {
            for k in 0..4 {
                let (p, q) = ([o[2 * k], o[2 * k + 1]], [b[2 * k], b[2 * k + 1]]);
                let r = if OD { pair(MUL, p, q) } else { pair(MUL, q, p) };
                o[2 * k] = r[0];
                o[2 * k + 1] = r[1];
            }
        }
        for (o, &b) in o8.into_remainder().iter_mut().zip(t8.remainder()) {
            *o = if OD { one(MUL, *o, b) } else { one(MUL, b, *o) };
        }
    }

    /// `own[i] = own[i] op k`, `own` the destination when `OD`.
    #[inline(always)]
    pub fn in_place_k<const MUL: bool, const OD: bool>(own: &mut [f64], k: f64) {
        let kk = [k, k];
        let mut o8 = own.chunks_exact_mut(8);
        for o in &mut o8 {
            for j in 0..4 {
                let p = [o[2 * j], o[2 * j + 1]];
                let r = if OD { pair(MUL, p, kk) } else { pair(MUL, kk, p) };
                o[2 * j] = r[0];
                o[2 * j + 1] = r[1];
            }
        }
        for o in o8.into_remainder() {
            *o = if OD { one(MUL, *o, k) } else { one(MUL, k, *o) };
        }
    }

    /// Dispatch a run-time (mul, destination) pair to the monomorphized
    /// kernels.
    macro_rules! dispatch {
        ($f:ident, $mul:expr, $dest:expr, $($arg:expr),*) => {
            match ($mul, $dest) {
                (false, false) => $f::<false, false>($($arg),*),
                (false, true) => $f::<false, true>($($arg),*),
                (true, false) => $f::<true, false>($($arg),*),
                (true, true) => $f::<true, true>($($arg),*),
            }
        };
    }
    pub(crate) use dispatch;
}

/// Which operand of `x + y` / `x * y` is the destination of GNU R's
/// compiled instruction (its NaN payload wins when both are NaN). From GNU
/// R 4.5.0's results (Debian r-base-core 4.5.0-3) over every shape:
/// doubles: `x` for scalar–scalar, `n2 == 1` and equal lengths, `y` for
/// `n1 == 1` and general recycling (R: arithmetic.c:real_binary's four
/// loops); an integer operand: `y` for scalar–scalar (do_arith's scalar
/// fast path), `x` otherwise (the MOD_ITERATE2 loop for mixed types).
/// Logical operands count as doubles (R coerces them first).
pub(super) fn x_is_dest(int_operand: bool, nx: usize, ny: usize) -> bool {
    if int_operand {
        !(nx == 1 && ny == 1)
    } else if ny == 1 {
        true
    } else if nx == 1 {
        false
    } else {
        nx == ny
    }
}

/// `+`/`*` on doubles (and integers as doubles) with GNU R's NaN payloads;
/// see [`exact`] and [`x_is_dest`].
pub(super) fn add_mul(mul: bool, a: &[f64], b: &[f64], n: usize, int_operand: bool) -> Vec<f64> {
    use exact::{append_vk, append_vv, dispatch};
    let mut out = Vec::with_capacity(n);
    if n == 0 {
        return out;
    }
    let x_dest = x_is_dest(int_operand, a.len(), b.len());
    let (na, nb) = (a.len(), b.len());
    if na == n && nb == n {
        dispatch!(append_vv, mul, x_dest, &mut out, a, b)
    } else if nb == 1 && na == n {
        dispatch!(append_vk, mul, x_dest, &mut out, a, b[0])
    } else if na == 1 && nb == n {
        // y is the vector: its operand is the destination unless x is.
        dispatch!(append_vk, mul, !x_dest, &mut out, b, a[0])
    } else if na == n && n % nb == 0 {
        // `b` recycled a whole number of times (a matrix times a column).
        out.reserve(n);
        for chunk in a.chunks(nb) {
            dispatch!(append_vv, mul, x_dest, &mut out, chunk, b)
        }
    } else if nb == n && n % na == 0 {
        out.reserve(n);
        for chunk in b.chunks(na) {
            dispatch!(append_vv, mul, x_dest, &mut out, a, chunk)
        }
    } else {
        out.reserve(n);
        let (mut i, mut j) = (0, 0);
        for _ in 0..n {
            out.push(if x_dest { exact::one(mul, a[i], b[j]) } else { exact::one(mul, b[j], a[i]) });
            i = if i + 1 == na { 0 } else { i + 1 };
            j = if j + 1 == nb { 0 } else { j + 1 };
        }
    }
    out
}
