//! The kernels (docs/g3.md §10, "one Rust implementation shared by the op
//! and the builtin object"): GNU R's arithmetic, comparison and logic on
//! plain data, with no R objects and no `Cx`. The builtins of this crate
//! run them over vectors (recycling, attributes, warnings and dispatch are
//! theirs); the VM's ops run the scalar functions on unboxed values (V1),
//! so the two can never disagree.
//!
//! - [`nan`]: `+` and `*` with the NaN payload GNU R's compiled code keeps
//!   (NA against NaN is observable).
//! - [`arith`]: arithmetic.c's integer and double operations and their
//!   recycling loops.
//! - [`complex`]: complex.c's operations over libgcc's `__muldc3`/`__divdc3`
//!   and glibc's `clog`/`cexp`.
//! - [`relop`] and [`logic`]: relop.c's and logic.c's element rules.

pub mod arith;
pub mod complex;
pub mod logic;
pub mod nan;
pub mod relop;

/// GNU R's `MOD_ITERATE2` (R_ext/Itermacros.h): `out[i] = f(xs[i1], ys[i2])`
/// with both operands recycled to `out.len()`, which is the longer length
/// (or 0). Equal lengths and a scalar operand take loops without index
/// wrapping; the result is the same.
#[inline(always)]
pub fn zip2<A: Copy, B: Copy, T>(xs: &[A], ys: &[B], out: &mut [T], mut f: impl FnMut(A, B) -> T) {
    let (n1, n2, n) = (xs.len(), ys.len(), out.len());
    if n == 0 {
        return;
    }
    debug_assert!(n1 > 0 && n2 > 0 && n == n1.max(n2));
    if n1 == n && n2 == n {
        for ((o, &x), &y) in out.iter_mut().zip(xs).zip(ys) {
            *o = f(x, y);
        }
    } else if n2 == 1 {
        let y = ys[0];
        for (o, &x) in out.iter_mut().zip(xs) {
            *o = f(x, y);
        }
    } else if n1 == 1 {
        let x = xs[0];
        for (o, &y) in out.iter_mut().zip(ys) {
            *o = f(x, y);
        }
    } else {
        let (mut i1, mut i2) = (0, 0);
        for o in out.iter_mut() {
            *o = f(xs[i1], ys[i2]);
            i1 += 1;
            if i1 == n1 {
                i1 = 0;
            }
            i2 += 1;
            if i2 == n2 {
                i2 = 0;
            }
        }
    }
}

/// The length of a binary operation's result: the longer operand's, or 0
/// when either is empty (arithmetic.c's "S4-compatibility change").
#[inline(always)]
pub fn result_len(n1: usize, n2: usize) -> usize {
    if n1 == 0 || n2 == 0 { 0 } else { n1.max(n2) }
}

/// Whether recycling `n1` against `n2` leaves a remainder: GNU R's
/// "longer object length is not a multiple of shorter object length".
#[inline(always)]
pub fn mismatch(n1: usize, n2: usize) -> bool {
    n1 > 0 && n2 > 0 && (if n1 > n2 { n1 % n2 } else { n2 % n1 }) != 0
}
