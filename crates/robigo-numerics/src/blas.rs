//! The reference BLAS routines GNU R's `%*%`, `crossprod` and `tcrossprod`
//! call for finite operands (array.c's `matprod` and friends, with
//! `options(matprod = "default")`): netlib's LAPACK 3.12.1 BLAS, which the
//! 4.6.2 oracle links (`/usr/lib/x86_64-linux-gnu/blas/libblas.so.3.12.1`;
//! GNU R ships the same Fortran as `src/extra/blas`). Only the forms
//! array.c uses, with ALPHA = 1 and BETA = 0 as it passes them, ported loop
//! by loop so that each sum is formed in the Fortran's order. A complex
//! product is Fortran's (gfortran's `-fcx-fortran-rules`: no rescue of a
//! NaN result), not C99's. Only finite operands reach these routines.
//!
//! The reference BLAS is under LAPACK's modified BSD licence
//! (`LICENSE-LAPACK` at the repository's root).

/// Which operands `dgemm`/`zgemm` transpose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trans {
    /// `C := A*B`
    NN,
    /// `C := A**T*B`
    TN,
    /// `C := A*B**T`
    NT,
}

/// `DGEMM(transa, transb, m, n, k, 1, a, lda, b, ldb, 0, c, ldc)`.
// reference BLAS 3.12.1: dgemm.f
#[allow(clippy::too_many_arguments)]
pub fn dgemm(
    t: Trans,
    m: usize,
    n: usize,
    k: usize,
    a: &[f64],
    lda: usize,
    b: &[f64],
    ldb: usize,
    c: &mut [f64],
    ldc: usize,
) {
    if m == 0 || n == 0 {
        return;
    }
    // (ALPHA == 1, so ALPHA*B(L,J) and ALPHA*TEMP are exact)
    match t {
        Trans::NN => {
            for j in 0..n {
                for i in 0..m {
                    c[i + j * ldc] = 0.0;
                }
                for l in 0..k {
                    let temp = b[l + j * ldb];
                    for i in 0..m {
                        c[i + j * ldc] += temp * a[i + l * lda];
                    }
                }
            }
        }
        Trans::TN => {
            for j in 0..n {
                for i in 0..m {
                    let mut temp = 0.0;
                    for l in 0..k {
                        temp += a[l + i * lda] * b[l + j * ldb];
                    }
                    c[i + j * ldc] = temp;
                }
            }
        }
        Trans::NT => {
            for j in 0..n {
                for i in 0..m {
                    c[i + j * ldc] = 0.0;
                }
                for l in 0..k {
                    let temp = b[j + l * ldb];
                    for i in 0..m {
                        c[i + j * ldc] += temp * a[i + l * lda];
                    }
                }
            }
        }
    }
}

/// `DGEMV(trans, m, n, 1, a, lda, x, 1, 0, y, 1)`, `trans` for 'T'.
// reference BLAS 3.12.1: dgemv.f
pub fn dgemv(trans: bool, m: usize, n: usize, a: &[f64], lda: usize, x: &[f64], y: &mut [f64]) {
    if m == 0 || n == 0 {
        return;
    }
    let leny = if trans { n } else { m };
    // First form y := beta*y (BETA == 0)
    for v in y.iter_mut().take(leny) {
        *v = 0.0;
    }
    if !trans {
        // Form y := alpha*A*x + y
        for j in 0..n {
            let temp = x[j];
            for i in 0..m {
                y[i] += temp * a[i + j * lda];
            }
        }
    } else {
        // Form y := alpha*A**T*x + y
        for j in 0..n {
            let mut temp = 0.0;
            for i in 0..m {
                temp += a[i + j * lda] * x[i];
            }
            y[j] += temp;
        }
    }
}

/// `DSYRK('U', trans, n, k, 1, a, lda, 0, c, ldc)`, `trans` for 'T': the
/// upper triangle of `A*A**T` (`A**T*A`).
// reference BLAS 3.12.1: dsyrk.f
pub fn dsyrk_upper(trans: bool, n: usize, k: usize, a: &[f64], lda: usize, c: &mut [f64], ldc: usize) {
    if n == 0 {
        return;
    }
    if !trans {
        // Form C := alpha*A*A**T + beta*C
        for j in 0..n {
            for i in 0..=j {
                c[i + j * ldc] = 0.0;
            }
            for l in 0..k {
                if a[j + l * lda] != 0.0 {
                    let temp = a[j + l * lda];
                    for i in 0..=j {
                        c[i + j * ldc] += temp * a[i + l * lda];
                    }
                }
            }
        }
    } else {
        // Form C := alpha*A**T*A + beta*C
        for j in 0..n {
            for i in 0..=j {
                let mut temp = 0.0;
                for l in 0..k {
                    temp += a[l + i * lda] * a[l + j * lda];
                }
                c[i + j * ldc] = temp;
            }
        }
    }
}

/// Fortran's `A*B` of two `DOUBLE COMPLEX` (no NaN rescue).
#[inline(always)]
fn zmul((ar, ai): (f64, f64), (br, bi): (f64, f64)) -> (f64, f64) {
    (ar * br - ai * bi, ar * bi + ai * br)
}

/// `ZGEMM(transa, transb, m, n, k, (1,0), a, lda, b, ldb, (0,0), c, ldc)`
/// over a complex type `T` read by `get` and made by `mk`.
// reference BLAS 3.12.1: zgemm.f
#[allow(clippy::too_many_arguments)]
pub fn zgemm<T: Copy>(
    t: Trans,
    (m, n, k): (usize, usize, usize),
    a: &[T],
    lda: usize,
    b: &[T],
    ldb: usize,
    c: &mut [T],
    ldc: usize,
    get: impl Fn(T) -> (f64, f64),
    mk: impl Fn(f64, f64) -> T,
) {
    if m == 0 || n == 0 {
        return;
    }
    let one = (1.0, 0.0);
    let zadd = |(ar, ai): (f64, f64), (br, bi): (f64, f64)| (ar + br, ai + bi);
    match t {
        Trans::NN | Trans::NT => {
            for j in 0..n {
                let mut col = vec![(0.0, 0.0); m];
                for l in 0..k {
                    let bl = if t == Trans::NN { b[l + j * ldb] } else { b[j + l * ldb] };
                    let temp = zmul(one, get(bl));
                    for (i, ci) in col.iter_mut().enumerate() {
                        *ci = zadd(*ci, zmul(temp, get(a[i + l * lda])));
                    }
                }
                for (i, ci) in col.into_iter().enumerate() {
                    c[i + j * ldc] = mk(ci.0, ci.1);
                }
            }
        }
        Trans::TN => {
            for j in 0..n {
                for i in 0..m {
                    let mut temp = (0.0, 0.0);
                    for l in 0..k {
                        temp = zadd(temp, zmul(get(a[l + i * lda]), get(b[l + j * ldb])));
                    }
                    let r = zmul(one, temp);
                    c[i + j * ldc] = mk(r.0, r.1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // GNU R 4.6.2's results (sprintf("%a")) for the matrices below, on the
    // oracle's reference BLAS (scripts in the PR that added this file)
    const MM: &[&str] = &[
        "0x1.20bb816b9ebc3p+2",
        "-0x1.21911ac694849p+3",
        "-0x1.dd8e6dd8e6dd7p-2",
        "-0x1.b5a1eb851fcb4p+9",
        "0x1.11f11eb851eb9p+11",
        "0x1.1bfd9b3222d7bp+6",
    ];
    const XTX: &[&str] = &[
        "0x1.da4fa4fa4fa51p+2",
        "0x1.5a35a362cca94p-1",
        "0x1.976d8976d8978p-2",
        "-0x1.feb851eb851ebp+0",
        "0x1.5a35a362cca94p-1",
        "0x1.e6f05397829ccp+4",
        "0x1.84725e66e33c9p+3",
        "-0x1.1e49247ed2fb1p+4",
        "0x1.976d8976d8978p-2",
        "0x1.84725e66e33c9p+3",
        "0x1.6a210144f8ce8p+2",
        "-0x1.b75fe235fe236p+3",
        "-0x1.feb851eb851ebp+0",
        "-0x1.1e49247ed2fb1p+4",
        "-0x1.b75fe235fe236p+3",
        "0x1.fe8p+5",
    ];
    const XXT: &[&str] = &[
        "0x1.ab0f5c28f5c29p+5",
        "-0x1.9df258b061f9bp+4",
        "0x1.ffc46c057ad86p+1",
        "-0x1.9df258b061f9bp+4",
        "0x1.70ba987654321p+5",
        "-0x1.4a9b101767dc8p-2",
        "0x1.ffc46c057ad86p+1",
        "-0x1.4a9b101767dc8p-2",
        "0x1.f5e24fa303418p+2",
    ];
    const XTY: &[&str] = &[
        "-0x1.745dc0bd53834p+0",
        "-0x1.e2a6c404a88bbp+2",
        "-0x1.92bc0b2bc0b2cp+1",
        "0x1.69718cdb5d11fp+2",
        "-0x1.51669d0369d03p+11",
        "0x1.ac4be2bf87c73p+8",
        "0x1.4e094f2094f2p+6",
        "0x1.4971eb851eb85p+9",
    ];
    const XYT: &[&str] = &[
        "0x1.0495f161c5234p+6",
        "-0x1.d35a35a35a35cp+2",
        "0x1.9cf2094f2094ep+2",
        "0x1.d6d3a08b4a9bbp+0",
        "-0x1.63628814065f9p-1",
        "0x1.0630c7963fac9p+0",
    ];
    const MV: &[&str] = &["0x1.c5bbbbb65d58p+4", "-0x1.6705b05b05b06p+4", "0x1.6c0d4c77b0352p-1"];
    const VM: &[&str] =
        &["-0x1.5aba453ded787p+2", "0x1.249249f2bad8p-4", "-0x1.9c2072e699779p-2", "0x1.ec57c57c57c58p+1"];
    const ZMM: &[&str] = &[
        "0x1.775d75d75d75fp+1",
        "-0x1.5fbc622c892fp+2",
        "-0x1.019ce075f6fd3p+3",
        "0x1.0399c06fa767bp+1",
        "-0x1.c6f8af8af8af9p+3",
        "0x1.ee2be2be2be2cp+4",
        "0x1.624962fc962fcp+5",
        "-0x1.aead65b7a3285p+3",
    ];
    const ZXTY: &[&str] = &[
        "-0x1.598f328cc265cp-2",
        "0x1.08f08f08f08f1p-1",
        "-0x1.36f235cb4c528p+3",
        "0x1.1fe76c8b43958p+2",
        "0x1.063f63f63f63fp+1",
        "-0x1.2618618618619p+1",
        "0x1.429eb851eb852p+5",
        "-0x1.f624dd2f1a9fcp+3",
    ];
    const ZXYT: &[&str] = &[
        "-0x1.cbd5bd5bd5bd5p+1",
        "-0x1.dda9da9da9dabp+0",
        "0x1.92f1a9fbe76c9p+0",
        "0x1.551cbf73380ep+2",
        "-0x1.c9d41d41d41d5p+3",
        "0x1.e936202ecfb9dp+4",
        "0x1.5ca8f5c28f5c2p+5",
        "-0x1.a9e098ead65b8p+3",
    ];

    /// A `%a` string's double.
    fn hexf(s: &str) -> f64 {
        let (neg, s) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
        let s = s.strip_prefix("0x").unwrap();
        let (mant, exp) = s.split_once('p').unwrap();
        let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
        let mut m = u64::from_str_radix(int, 16).unwrap() as f64;
        let mut scale = 1.0 / 16.0;
        for d in frac.chars() {
            m += d.to_digit(16).unwrap() as f64 * scale;
            scale /= 16.0;
        }
        let v = m * 2f64.powi(exp.parse().unwrap());
        if neg { -v } else { v }
    }

    fn same(got: &[f64], want: &[&str]) {
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g.to_bits(), hexf(w).to_bits(), "{g:e} against {w}");
        }
    }

    const A: [f64; 12] = [0.1, 1.0 / 3.0, -2.7, 1e-8, 5.5, 3.0 / 7.0, -0.9, 2.2, 1.0 / 11.0, 7.25, -3.3, 0.6];
    const B: [f64; 8] = [1.0 / 9.0, -1.4, 0.37, 2.0 / 3.0, 8.1, -0.05, 1e3, 3.3];

    #[test]
    fn dgemm_and_dgemv_give_the_oracles_products() {
        let mut c = [0.0; 6];
        dgemm(Trans::NN, 3, 2, 4, &A, 3, &B, 4, &mut c, 3);
        same(&c, MM);
        // crossprod(a, b[1:3, ])
        let b3 = [B[0], B[1], B[2], B[4], B[5], B[6]];
        let mut c = [0.0; 8];
        dgemm(Trans::TN, 4, 2, 3, &A, 3, &b3, 3, &mut c, 4);
        same(&c, XTY);
        // tcrossprod(a, y), y 2 x 4
        let y = [0.25, -1.0 / 3.0, 4.0, 0.7, 1.0 / 7.0, -2.0, 9.0, 0.01];
        let mut c = [0.0; 6];
        dgemm(Trans::NT, 3, 2, 4, &A, 3, &y, 2, &mut c, 3);
        same(&c, XYT);
        let mut v = [0.0; 3];
        dgemv(false, 3, 4, &A, 3, &[1.0 / 3.0, -2.0, 0.75, 4.0], &mut v);
        same(&v, MV);
        let mut v = [0.0; 4];
        dgemv(true, 3, 4, &A, 3, &[0.3, -1.0 / 7.0, 2.0], &mut v);
        same(&v, VM);
    }

    #[test]
    fn dsyrk_gives_the_oracles_triangle() {
        let mirror = |c: &mut [f64], n: usize| {
            for i in 1..n {
                for j in 0..i {
                    c[i + n * j] = c[j + n * i];
                }
            }
        };
        let mut c = [0.0; 16];
        dsyrk_upper(true, 4, 3, &A, 3, &mut c, 4);
        mirror(&mut c, 4);
        same(&c, XTX);
        let mut c = [0.0; 9];
        dsyrk_upper(false, 3, 4, &A, 3, &mut c, 3);
        mirror(&mut c, 3);
        same(&c, XXT);
    }

    #[test]
    fn zgemm_gives_the_oracles_products() {
        let z = [(0.1, 1.0 / 7.0), (1.0 / 3.0, -0.2), (-2.7, 3.0), (5.5, 1e-3)];
        let w = [(2.0 / 3.0, -1.0 / 9.0), (-1.5, 0.4), (0.25, 1.0), (8.0, -2.5)];
        for (t, want) in [(Trans::NN, ZMM), (Trans::TN, ZXTY), (Trans::NT, ZXYT)] {
            let mut c = [(0.0, 0.0); 4];
            zgemm(t, (2, 2, 2), &z, 2, &w, 2, &mut c, 2, |x| x, |r, i| (r, i));
            // (R printed each element's real and imaginary parts in turn)
            let got: Vec<f64> = c.iter().flat_map(|x| [x.0, x.1]).collect();
            same(&got, want);
        }
    }
}
