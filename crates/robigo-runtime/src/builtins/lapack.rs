//! LAPACK-based linear algebra internals (GNU R's modules/lapack/Lapack.c).
//!
//! Real eigen/SVD, pivoted Cholesky and the complex eigen/SVD/solve paths
//! bind the system LAPACK (liblapack.so.3) lazily on first use, exactly as
//! GNU R does, so results match the oracle bit for bit. The remaining
//! LAPACK and reference BLAS routines are ported from the Fortran sources
//! R ships (`src/modules/lapack/dlapack.f`, `src/extra/blas/blas.f`),
//! keeping their operation order so results agree with R linked against
//! the reference libraries. Matrices are column-major slices with a
//! leading dimension, as in Fortran.

use super::{arg, internal};
use crate::attrib::set_attrib;
use crate::coerce::{as_integer, as_logical, as_real, coerce_quiet};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

use super::array::dtrsm_left;

pub static DEFS: &[Builtin] = &[
    internal("La_rs", do_la_rs, 2),
    internal("La_rs_cmplx", do_la_rs_cmplx, 2),
    internal("La_rg", do_la_rg, 2),
    internal("La_rg_cmplx", do_la_rg_cmplx, 2),
    internal("La_svd", do_la_svd, 5),
    internal("La_svd_cmplx", do_la_svd_cmplx, 5),
    internal("La_solve", do_la_solve, 3),
    internal("La_solve_cmplx", do_la_solve_cmplx, 3),
    internal("det_ge_real", do_det_ge_real, 2),
    internal("La_chol", do_la_chol, 3),
    internal("La_chol2inv", do_la_chol2inv, 2),
    internal("La_dgecon", do_la_dgecon, 2),
    internal("La_dlange", do_la_dlange, 2),
    internal("La_version", do_la_version, 0),
    internal("La_library", do_la_library, 0),
];

// ----------------------------------------------------------------------
// Reference BLAS / LAPACK kernels

/// DLAMCH('S'): safe minimum.
const SFMIN: f64 = f64::MIN_POSITIVE;

/// IDAMAX: 0-based index of the first element of maximal |x|.
fn idamax(x: &[f64]) -> usize {
    if x.is_empty() {
        return 0;
    }
    let mut imax = 0;
    let mut dmax = x[0].abs();
    for (k, v) in x.iter().enumerate().skip(1) {
        if v.abs() > dmax {
            imax = k;
            dmax = v.abs();
        }
    }
    imax
}

fn dasum(x: &[f64]) -> f64 {
    let mut s = 0.0;
    for v in x {
        s += v.abs();
    }
    s
}

/// DLASWP: apply row interchanges `ipiv[k1..=k2]` (0-based rows, pivots
/// 0-based) to the `ncols` columns of `a` starting at column `c0`.
fn dlaswp(a: &mut [f64], lda: usize, c0: usize, ncols: usize, k1: usize, k2: usize, ipiv: &[usize]) {
    for i in k1..=k2 {
        let ip = ipiv[i];
        if ip != i {
            for c in c0..c0 + ncols {
                a.swap(i + c * lda, ip + c * lda);
            }
        }
    }
}

/// DGEMM('N','N', m, n, k, -1, A, B, 1, C): C -= A %*% B on sub-blocks
/// given by (row, col) offsets into the same column-major array.
#[allow(clippy::too_many_arguments)]
fn dgemm_minus(
    a: &mut [f64],
    lda: usize,
    m: usize,
    n: usize,
    k: usize,
    ar: usize,
    ac: usize,
    br: usize,
    bc: usize,
    cr: usize,
    cc: usize,
) {
    for j in 0..n {
        for l in 0..k {
            let temp = -a[(br + l) + (bc + j) * lda];
            for i in 0..m {
                let v = temp * a[(ar + i) + (ac + l) * lda];
                a[(cr + i) + (cc + j) * lda] += v;
            }
        }
    }
}

/// DTRSM('L','L','N','U', m, n, 1, A, B) on sub-blocks of one array.
#[allow(clippy::too_many_arguments)]
fn dtrsm_llnu_inplace(a: &mut [f64], lda: usize, m: usize, n: usize, ar: usize, ac: usize, br: usize, bc: usize) {
    for j in 0..n {
        for k in 0..m {
            let bkj = a[(br + k) + (bc + j) * lda];
            if bkj != 0.0 {
                for i in k + 1..m {
                    let v = bkj * a[(ar + i) + (ac + k) * lda];
                    a[(br + i) + (bc + j) * lda] -= v;
                }
            }
        }
    }
}

/// DGETRF2 (recursive LU with partial pivoting) on the m x n block of `a`
/// at (r0, c0). Pivots are stored 0-based relative to the block. Returns
/// INFO (0, or the 1-based index of the first zero pivot).
fn dgetrf2(a: &mut [f64], lda: usize, r0: usize, c0: usize, m: usize, n: usize, ipiv: &mut [usize]) -> usize {
    if m == 0 || n == 0 {
        return 0;
    }
    let at = |r: usize, c: usize| (r0 + r) + (c0 + c) * lda;
    if m == 1 {
        ipiv[0] = 0;
        return if a[at(0, 0)] == 0.0 { 1 } else { 0 };
    }
    if n == 1 {
        let col: Vec<f64> = (0..m).map(|r| a[at(r, 0)]).collect();
        let i = idamax(&col);
        ipiv[0] = i;
        if a[at(i, 0)] != 0.0 {
            if i != 0 {
                a.swap(at(0, 0), at(i, 0));
            }
            let piv = a[at(0, 0)];
            if piv.abs() >= SFMIN {
                let rec = 1.0 / piv;
                for r in 1..m {
                    a[at(r, 0)] *= rec;
                }
            } else {
                for r in 1..m {
                    a[at(r, 0)] /= piv;
                }
            }
            return 0;
        }
        return 1;
    }
    let n1 = m.min(n) / 2;
    let n2 = n - n1;
    let mut info = 0;
    let iinfo = dgetrf2(a, lda, r0, c0, m, n1, &mut ipiv[..n1]);
    if info == 0 && iinfo > 0 {
        info = iinfo;
    }
    // Apply interchanges to [A12; A22].
    {
        let piv: Vec<usize> = ipiv[..n1].to_vec();
        for (k, &p) in piv.iter().enumerate() {
            if p != k {
                for c in n1..n {
                    a.swap(at(k, c), at(p, c));
                }
            }
        }
    }
    // Solve A12.
    dtrsm_llnu_inplace(a, lda, n1, n2, r0, c0, r0, c0 + n1);
    // Update A22.
    dgemm_minus(a, lda, m - n1, n2, n1, r0 + n1, c0, r0, c0 + n1, r0 + n1, c0 + n1);
    // Factor A22.
    let iinfo = dgetrf2(a, lda, r0 + n1, c0 + n1, m - n1, n2, &mut ipiv[n1..m.min(n)]);
    if info == 0 && iinfo > 0 {
        info = iinfo + n1;
    }
    for p in ipiv[n1..m.min(n)].iter_mut() {
        *p += n1;
    }
    // Apply interchanges to A21.
    let piv: Vec<usize> = ipiv[n1..m.min(n)].to_vec();
    for (k, &p) in piv.iter().enumerate() {
        let k = k + n1;
        if p != k {
            for c in 0..n1 {
                a.swap(at(k, c), at(p, c));
            }
        }
    }
    info
}

/// DGETRF: blocked LU factorization (block size 64, as ILAENV gives).
fn dgetrf(a: &mut [f64], lda: usize, m: usize, n: usize, ipiv: &mut [usize]) -> usize {
    const NB: usize = 64;
    let mn = m.min(n);
    if mn == 0 {
        return 0;
    }
    if NB >= mn {
        return dgetrf2(a, lda, 0, 0, m, n, ipiv);
    }
    let mut info = 0;
    let mut j = 0;
    while j < mn {
        let jb = (mn - j).min(NB);
        let iinfo = dgetrf2(a, lda, j, j, m - j, jb, &mut ipiv[j..j + jb]);
        if info == 0 && iinfo > 0 {
            info = iinfo + j;
        }
        for p in ipiv[j..(j + jb).min(m)].iter_mut() {
            *p += j;
        }
        dlaswp(a, lda, 0, j, j, j + jb - 1, ipiv);
        if j + jb < n {
            dlaswp(a, lda, j + jb, n - j - jb, j, j + jb - 1, ipiv);
            dtrsm_llnu_inplace(a, lda, jb, n - j - jb, j, j, j, j + jb);
            if j + jb < m {
                dgemm_minus(a, lda, m - j - jb, n - j - jb, jb, j + jb, j, j, j + jb, j + jb, j + jb);
            }
        }
        j += NB;
    }
    info
}

/// DGETRS('N'): solve A X = B with the LU factors of A.
fn dgetrs(lu: &[f64], n: usize, ipiv: &[usize], b: &mut [f64], nrhs: usize) {
    if n == 0 || nrhs == 0 {
        return;
    }
    dlaswp(b, n, 0, nrhs, 0, n - 1, ipiv);
    dtrsm_left(false, false, true, n, nrhs, lu, n, b, n);
    dtrsm_left(true, false, false, n, nrhs, lu, n, b, n);
}

/// DLANGE('1'): maximum absolute column sum.
fn dlange_one(a: &[f64], m: usize, n: usize) -> f64 {
    let mut value: f64 = 0.0;
    for j in 0..n {
        let mut sum = 0.0;
        for i in 0..m {
            sum += a[i + j * m].abs();
        }
        if value < sum || sum.is_nan() {
            value = sum;
        }
    }
    value
}

/// DLANGE('I'): maximum absolute row sum.
fn dlange_inf(a: &[f64], m: usize, n: usize) -> f64 {
    let mut work = vec![0.0; m];
    for j in 0..n {
        for i in 0..m {
            work[i] += a[i + j * m].abs();
        }
    }
    let mut value: f64 = 0.0;
    for &t in &work {
        if value < t || t.is_nan() {
            value = t;
        }
    }
    value
}

/// DTRSV (incx = 1).
fn dtrsv(upper: bool, trans: bool, unit: bool, n: usize, a: &[f64], lda: usize, x: &mut [f64]) {
    if !trans {
        if upper {
            for j in (0..n).rev() {
                if x[j] != 0.0 {
                    if !unit {
                        x[j] /= a[j + j * lda];
                    }
                    let temp = x[j];
                    for i in (0..j).rev() {
                        x[i] -= temp * a[i + j * lda];
                    }
                }
            }
        } else {
            for j in 0..n {
                if x[j] != 0.0 {
                    if !unit {
                        x[j] /= a[j + j * lda];
                    }
                    let temp = x[j];
                    for i in j + 1..n {
                        x[i] -= temp * a[i + j * lda];
                    }
                }
            }
        }
    } else if upper {
        for j in 0..n {
            let mut temp = x[j];
            for i in 0..j {
                temp -= a[i + j * lda] * x[i];
            }
            if !unit {
                temp /= a[j + j * lda];
            }
            x[j] = temp;
        }
    } else {
        for j in (0..n).rev() {
            let mut temp = x[j];
            for i in (j + 1..n).rev() {
                temp -= a[i + j * lda] * x[i];
            }
            if !unit {
                temp /= a[j + j * lda];
            }
            x[j] = temp;
        }
    }
}

/// DLATRS on the path taken for matrices without extreme scaling: the
/// column norms are computed and the system solved with DTRSV. The
/// overflow-guarded path for badly scaled matrices is not ported.
fn dlatrs(upper: bool, trans: bool, unit: bool, n: usize, a: &[f64], lda: usize, x: &mut [f64]) -> f64 {
    if n == 0 {
        return 1.0;
    }
    dtrsv(upper, trans, unit, n, a, lda, x);
    1.0
}

/// DGECON: estimate the reciprocal condition number of a matrix from its
/// LU factorization (`onenrm`: 1-norm, else infinity-norm).
fn dgecon(onenrm: bool, n: usize, lu: &[f64], anorm: f64) -> f64 {
    if n == 0 {
        return 1.0;
    }
    if anorm == 0.0 {
        return 0.0;
    }
    if anorm.is_nan() {
        return anorm;
    }
    if anorm > f64::MAX {
        return 0.0;
    }
    let kase1 = if onenrm { 1 } else { 2 };
    let mut est = DLacn2::new(n);
    let mut x = vec![0.0; n];
    let mut kase = 0;
    loop {
        kase = est.step(&mut x, kase);
        if kase == 0 {
            break;
        }
        let (sl, su);
        if kase == kase1 {
            sl = dlatrs(false, false, true, n, lu, n, &mut x);
            su = dlatrs(true, false, false, n, lu, n, &mut x);
        } else {
            su = dlatrs(true, true, false, n, lu, n, &mut x);
            sl = dlatrs(false, true, true, n, lu, n, &mut x);
        }
        let scale = sl * su;
        if scale != 1.0 {
            let ix = idamax(&x);
            if scale < x[ix].abs() * SFMIN || scale == 0.0 {
                return 0.0;
            }
            for v in x.iter_mut() {
                *v /= scale;
            }
        }
    }
    let ainvnm = est.est;
    if ainvnm != 0.0 { (1.0 / ainvnm) / anorm } else { 0.0 }
}

/// DLACN2: Higham's estimate of the 1-norm of a matrix, by reverse
/// communication.
struct DLacn2 {
    n: usize,
    v: Vec<f64>,
    isgn: Vec<i64>,
    est: f64,
    isave: [usize; 3],
}

impl DLacn2 {
    fn new(n: usize) -> Self {
        DLacn2 { n, v: vec![0.0; n], isgn: vec![0; n], est: 0.0, isave: [0; 3] }
    }

    fn sign_vec(&mut self, x: &mut [f64]) {
        for k in 0..self.n {
            x[k] = if x[k] >= 0.0 { 1.0 } else { -1.0 };
            self.isgn[k] = x[k].round() as i64;
        }
    }

    fn unit_vec(&mut self, x: &mut [f64]) -> i32 {
        for v in x.iter_mut() {
            *v = 0.0;
        }
        x[self.isave[1]] = 1.0;
        self.isave[0] = 3;
        1
    }

    fn altsgn(&mut self, x: &mut [f64]) -> i32 {
        let n = self.n;
        let mut altsgn = 1.0;
        for k in 0..n {
            x[k] = altsgn * (1.0 + k as f64 / (n - 1) as f64);
            altsgn = -altsgn;
        }
        self.isave[0] = 5;
        1
    }

    /// One call of DLACN2; returns the new KASE.
    fn step(&mut self, x: &mut [f64], kase: i32) -> i32 {
        let n = self.n;
        if kase == 0 {
            for v in x.iter_mut() {
                *v = 1.0 / n as f64;
            }
            self.isave[0] = 1;
            return 1;
        }
        match self.isave[0] {
            1 => {
                if n == 1 {
                    self.v[0] = x[0];
                    self.est = self.v[0].abs();
                    return 0;
                }
                self.est = dasum(x);
                self.sign_vec(x);
                self.isave[0] = 2;
                2
            }
            2 => {
                self.isave[1] = idamax(x);
                self.isave[2] = 2;
                self.unit_vec(x)
            }
            3 => {
                self.v.copy_from_slice(x);
                let estold = self.est;
                self.est = dasum(&self.v);
                let mut changed = false;
                for k in 0..n {
                    let xs: f64 = if x[k] >= 0.0 { 1.0 } else { -1.0 };
                    if xs.round() as i64 != self.isgn[k] {
                        changed = true;
                        break;
                    }
                }
                if !changed || self.est <= estold {
                    return self.altsgn(x);
                }
                self.sign_vec(x);
                self.isave[0] = 4;
                2
            }
            4 => {
                let jlast = self.isave[1];
                self.isave[1] = idamax(x);
                if x[jlast] != x[self.isave[1]].abs() && self.isave[2] < 5 {
                    self.isave[2] += 1;
                    return self.unit_vec(x);
                }
                self.altsgn(x)
            }
            _ => {
                let temp = 2.0 * (dasum(x) / (3 * n) as f64);
                if temp > self.est {
                    self.v.copy_from_slice(x);
                    self.est = temp;
                }
                0
            }
        }
    }
}

/// DSYRK('U','T', n, k, -1, A, 1, C) on sub-blocks of one array.
#[allow(clippy::too_many_arguments)]
fn dsyrk_ut_minus(a: &mut [f64], lda: usize, n: usize, k: usize, ar: usize, ac: usize, cr: usize, cc: usize) {
    for j in 0..n {
        for i in 0..=j {
            let mut temp = 0.0;
            for l in 0..k {
                temp += a[(ar + l) + (ac + i) * lda] * a[(ar + l) + (ac + j) * lda];
            }
            let c = (cr + i) + (cc + j) * lda;
            a[c] += -temp;
        }
    }
}

/// DTRSM('L','U','T','N', m, n, 1, A, B) on sub-blocks of one array.
#[allow(clippy::too_many_arguments)]
fn dtrsm_lutn_inplace(a: &mut [f64], lda: usize, m: usize, n: usize, ar: usize, ac: usize, br: usize, bc: usize) {
    for j in 0..n {
        for i in 0..m {
            let mut temp = a[(br + i) + (bc + j) * lda];
            for k in 0..i {
                temp -= a[(ar + k) + (ac + i) * lda] * a[(br + k) + (bc + j) * lda];
            }
            temp /= a[(ar + i) + (ac + i) * lda];
            a[(br + i) + (bc + j) * lda] = temp;
        }
    }
}

/// DPOTRF2('U') (recursive Cholesky) on the n x n block at (r0, r0).
fn dpotrf2_upper(a: &mut [f64], lda: usize, r0: usize, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    if n == 1 {
        let v = a[r0 + r0 * lda];
        if v <= 0.0 || v.is_nan() {
            return 1;
        }
        a[r0 + r0 * lda] = v.sqrt();
        return 0;
    }
    let n1 = n / 2;
    let n2 = n - n1;
    let iinfo = dpotrf2_upper(a, lda, r0, n1);
    if iinfo != 0 {
        return iinfo;
    }
    dtrsm_lutn_inplace(a, lda, n1, n2, r0, r0, r0, r0 + n1);
    dsyrk_ut_minus(a, lda, n2, n1, r0, r0 + n1, r0 + n1, r0 + n1);
    let iinfo = dpotrf2_upper(a, lda, r0 + n1, n2);
    if iinfo != 0 {
        return iinfo + n1;
    }
    0
}

/// DPOTRF('U') with ILAENV's block size 64.
fn dpotrf_upper(a: &mut [f64], n: usize) -> usize {
    const NB: usize = 64;
    if NB >= n {
        return dpotrf2_upper(a, n, 0, n);
    }
    let lda = n;
    let mut j = 0;
    while j < n {
        let jb = NB.min(n - j);
        dsyrk_ut_minus(a, lda, jb, j, 0, j, j, j);
        let info = dpotrf2_upper(a, lda, j, jb);
        if info != 0 {
            return info + j;
        }
        if j + jb < n {
            // DGEMM('T','N', jb, n-j-jb, j, -1, A(1,j), A(1,j+jb), 1, A(j,j+jb))
            for c in 0..n - j - jb {
                for r in 0..jb {
                    let mut temp = 0.0;
                    for l in 0..j {
                        temp += a[l + (j + r) * lda] * a[l + (j + jb + c) * lda];
                    }
                    a[(j + r) + (j + jb + c) * lda] += -temp;
                }
            }
            dtrsm_lutn_inplace(a, lda, jb, n - j - jb, j, j, j, j + jb);
        }
        j += NB;
    }
    0
}

/// DPOTRI('U') via DTRTI2 and DLAUU2 (unblocked; R's sizes are small).
fn dpotri_upper(a: &mut [f64], n: usize) -> usize {
    let lda = n;
    // DTRTRI: check for singularity
    for j in 0..n {
        if a[j + j * lda] == 0.0 {
            return j + 1;
        }
    }
    // DTRTI2('U','N')
    for j in 0..n {
        a[j + j * lda] = 1.0 / a[j + j * lda];
        let ajj = -a[j + j * lda];
        // DTRMV('U','N','N', j, A, x = A(1:j, j))
        for jj in 0..j {
            let xj = a[jj + j * lda];
            if xj != 0.0 {
                let temp = xj;
                for i in 0..jj {
                    let v = temp * a[i + jj * lda];
                    a[i + j * lda] += v;
                }
                a[jj + j * lda] *= a[jj + jj * lda];
            }
        }
        for i in 0..j {
            a[i + j * lda] *= ajj;
        }
    }
    // DLAUU2('U'): U * U**T
    for i in 0..n {
        let aii = a[i + i * lda];
        if i + 1 < n {
            let mut dot = 0.0;
            for k in i..n {
                dot += a[i + k * lda] * a[i + k * lda];
            }
            a[i + i * lda] = dot;
            // DGEMV('N', i, n-i-1, 1, A(1,i+1), x = A(i,i+1) stride lda, beta = aii, y = A(1,i))
            for r in 0..i {
                a[r + i * lda] *= aii;
            }
            for c in i + 1..n {
                let temp = a[i + c * lda];
                for r in 0..i {
                    let v = temp * a[r + c * lda];
                    a[r + i * lda] += v;
                }
            }
        } else {
            for r in 0..=i {
                a[r + i * lda] *= aii;
            }
        }
    }
    0
}

// ----------------------------------------------------------------------
// System LAPACK bridge
//
// GNU R's lapack module (src/modules/lapack) links the system LAPACK
// (liblapack.so.3) and calls its Fortran ABI directly. robigo binds the
// same shared library lazily on the first call, so eigen()/svd()/chol()
// results — and La_library()/La_version() — match the oracle running on
// the same machine bit for bit, and startup pays nothing until then.
// Fortran string arguments carry hidden trailing length parameters
// (FCONE) on this ABI. The handle is never closed: the resolved function
// pointers stay valid for the process lifetime.

type DsyevrFn = unsafe extern "C" fn(
    *const u8,
    *const u8,
    *const u8,
    *const i32,
    *mut f64,
    *const i32,
    *const f64,
    *const f64,
    *const i32,
    *const i32,
    *const f64,
    *mut i32,
    *mut f64,
    *mut f64,
    *const i32,
    *mut i32,
    *mut f64,
    *const i32,
    *mut i32,
    *const i32,
    *mut i32,
    usize,
    usize,
    usize,
);
type DgeevFn = unsafe extern "C" fn(
    *const u8,
    *const u8,
    *const i32,
    *mut f64,
    *const i32,
    *mut f64,
    *mut f64,
    *mut f64,
    *const i32,
    *mut f64,
    *const i32,
    *mut f64,
    *const i32,
    *mut i32,
    usize,
    usize,
);
type DgesddFn = unsafe extern "C" fn(
    *const u8,
    *const i32,
    *const i32,
    *mut f64,
    *const i32,
    *mut f64,
    *mut f64,
    *const i32,
    *mut f64,
    *const i32,
    *mut f64,
    *const i32,
    *mut i32,
    *mut i32,
    usize,
);
type DpstrfFn = unsafe extern "C" fn(
    *const u8,
    *const i32,
    *mut f64,
    *const i32,
    *mut i32,
    *mut i32,
    *const f64,
    *mut f64,
    *mut i32,
    usize,
);
type IlaverFn = unsafe extern "C" fn(*mut i32, *mut i32, *mut i32);
type ZheevFn = unsafe extern "C" fn(
    *const u8,
    *const u8,
    *const i32,
    *mut Z,
    *const i32,
    *mut f64,
    *mut Z,
    *const i32,
    *mut f64,
    *mut i32,
    usize,
    usize,
);
type ZgeevFn = unsafe extern "C" fn(
    *const u8,
    *const u8,
    *const i32,
    *mut Z,
    *const i32,
    *mut Z,
    *mut Z,
    *const i32,
    *mut Z,
    *const i32,
    *mut Z,
    *const i32,
    *mut f64,
    *mut i32,
    usize,
    usize,
);
type ZgesddFn = unsafe extern "C" fn(
    *const u8,
    *const i32,
    *const i32,
    *mut Z,
    *const i32,
    *mut f64,
    *mut Z,
    *const i32,
    *mut Z,
    *const i32,
    *mut Z,
    *const i32,
    *mut f64,
    *mut i32,
    *mut i32,
    usize,
);
type ZgesvFn = unsafe extern "C" fn(*const i32, *const i32, *mut Z, *const i32, *mut i32, *mut Z, *const i32, *mut i32);
type ZgeconFn = unsafe extern "C" fn(
    *const u8,
    *const i32,
    *const Z,
    *const i32,
    *const f64,
    *mut f64,
    *mut Z,
    *mut f64,
    *mut i32,
    usize,
);
type ZlangeFn = unsafe extern "C" fn(*const u8, *const i32, *const i32, *const Z, *const i32, *mut Z, usize) -> f64;

/// Rcomplex on the C side: two doubles in declaration order (r, i).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Z {
    r: f64,
    i: f64,
}

const _: () = assert!(std::mem::size_of::<Z>() == 16);

struct SystemLapack {
    dsyevr: DsyevrFn,
    dgeev: DgeevFn,
    dgesdd: DgesddFn,
    dpstrf: DpstrfFn,
    ilaver: IlaverFn,
    zheev: ZheevFn,
    zgeev: ZgeevFn,
    zgesdd: ZgesddFn,
    zgesv: ZgesvFn,
    zgecon: ZgeconFn,
    zlange: ZlangeFn,
}

/// The dlsym-resolved address of the bound library's ilaver entry point,
/// for dladdr in La_library.
static ILAVER_PTR: std::sync::OnceLock<SendPtr> = std::sync::OnceLock::new();

#[derive(Clone, Copy)]
struct SendPtr(*mut libc::c_void);
// SAFETY: an opaque dlsym address that is only read (via dladdr) after the
// process-wide OnceLock initialization has completed.
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// dlopen the system LAPACK and resolve the routines R's lapack module
/// uses. `RTLD_LAZY` keeps the first-call cost to the routines actually
/// reached, mirroring how little of the library these paths touch.
fn load_system_lapack() -> Result<SystemLapack, &'static str> {
    const NAMES: [&[u8]; 3] = [b"liblapack.so.3\0", b"liblapack.so\0", b"libRlapack.so\0"];
    let mut handle = std::ptr::null_mut();
    for name in &NAMES {
        // SAFETY: `name` is NUL-terminated; dlopen is thread-safe.
        let h = unsafe { libc::dlopen(name.as_ptr().cast(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
        if !h.is_null() {
            handle = h;
            break;
        }
    }
    if handle.is_null() {
        return Err("robigo: system LAPACK library (liblapack.so.3) is unavailable");
    }
    // SAFETY: the symbol table is immutable once the library is loaded;
    // the transmuted pointers are only ever called with matching signatures.
    unsafe fn sym<T: Copy>(handle: *mut libc::c_void, name: &[u8]) -> Result<T, &'static str> {
        let p = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
        if p.is_null() {
            return Err("robigo: system LAPACK is missing a required routine");
        }
        Ok(unsafe { std::mem::transmute_copy::<*mut libc::c_void, T>(&p) })
    }
    unsafe {
        let ilaver: IlaverFn = sym(handle, b"ilaver_\0")?;
        let ilaver_ptr: *mut libc::c_void = libc::dlsym(handle, b"ilaver_\0".as_ptr().cast());
        let _ = ILAVER_PTR.set(SendPtr(ilaver_ptr));
        let lib = SystemLapack {
            dsyevr: sym(handle, b"dsyevr_\0")?,
            dgeev: sym(handle, b"dgeev_\0")?,
            dgesdd: sym(handle, b"dgesdd_\0")?,
            dpstrf: sym(handle, b"dpstrf_\0")?,
            ilaver,
            zheev: sym(handle, b"zheev_\0")?,
            zgeev: sym(handle, b"zgeev_\0")?,
            zgesdd: sym(handle, b"zgesdd_\0")?,
            zgesv: sym(handle, b"zgesv_\0")?,
            zgecon: sym(handle, b"zgecon_\0")?,
            zlange: sym(handle, b"zlange_\0")?,
        };
        Ok(lib)
    }
}

fn system_lapack() -> RResult<&'static SystemLapack> {
    static LIB: std::sync::OnceLock<Result<SystemLapack, &'static str>> = std::sync::OnceLock::new();
    LIB.get_or_init(load_system_lapack).as_ref().map_err(|msg| Flow::error(*msg))
}

// ----------------------------------------------------------------------
// The internals

fn is_numeric_matrix(a: &Value) -> bool {
    matches!(a.get_attr(s::DIM), Some(d) if d.len() == 2) && matches!(a, Value::Real(_) | Value::Int(_) | Value::Lgl(_))
}

fn dims2(a: &Value) -> (usize, usize) {
    match coerce_quiet(&a.get_attr(s::DIM).unwrap_or(Value::Null), VType::Int) {
        Value::Int(d) if d.data.len() == 2 => (d.data[0] as usize, d.data[1] as usize),
        _ => (0, 0),
    }
}

fn reals(v: &Value) -> Vec<f64> {
    match coerce_quiet(v, VType::Real) {
        Value::Real(r) => r.data.clone(),
        _ => vec![],
    }
}

/// The complex data of `v`, coerced to complex as Lapack.c's coerceVector
/// does (quietly, keeping NA). Missing/empty becomes empty.
fn cplx_data(v: &Value) -> Vec<Z> {
    match coerce_quiet(v, VType::Cplx) {
        Value::Cplx(c) => c.data.iter().map(|z| Z { r: z.re, i: z.im }).collect(),
        _ => vec![],
    }
}

fn z_to_value(z: Vec<Z>) -> Value {
    Value::cplxs(z.into_iter().map(|z| Complex::new(z.r, z.i)).collect())
}

fn dimnames_elt(v: &Value, k: usize) -> Value {
    match v.get_attr(s::DIMNAMES) {
        Some(Value::List(l)) => l.data.get(k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// `%g` formatting as C's printf does it.
fn fmt_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    robigo_syntax::cfmt::fmt_g(x, 6, false, false)
}

/// R: Lapack.c:La_rs
fn do_la_rs(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let (n, n2) = dims2(&x);
    if n != n2 {
        return Err(Flow::error("'x' must be a square numeric matrix"));
    }
    let ov = as_logical(&arg(&args, 1));
    if ov == NA_LOGICAL {
        return Err(Flow::error("invalid 'only.values' argument"));
    }
    let n_i = n as i32;
    // LAPACK trashes its first argument, so hand it a copy as R does.
    let mut rx = reals(&x);
    let mut values = vec![0.0f64; n];
    let mut z = vec![0.0f64; if ov == 0 { n * n } else { 0 }];
    let mut isuppz = vec![0i32; 2 * n];
    let jobv: u8 = if ov != 0 { b'N' } else { b'V' };
    let zp: *mut f64 = if ov == 0 { z.as_mut_ptr() } else { std::ptr::null_mut() };
    let f = system_lapack()?.dsyevr;
    let mut m = 0i32;
    let mut info = 0i32;
    let mut tmp = 0.0f64;
    let mut itmp = 0i32;
    // Workspace query (lwork = liwork = -1), then the real call.
    // R: Lapack.c:La_rs leaves il/iu unset; RANGE='A' ignores them.
    // SAFETY: every pointer refers to a live allocation of the documented
    // size for the duration of the call.
    unsafe {
        f(
            &jobv,
            &b'A',
            &b'L',
            &n_i,
            rx.as_mut_ptr(),
            &n_i,
            &0.0,
            &0.0,
            &0,
            &0,
            &0.0,
            &mut m,
            values.as_mut_ptr(),
            zp,
            &n_i,
            isuppz.as_mut_ptr(),
            &mut tmp,
            &-1,
            &mut itmp,
            &-1,
            &mut info,
            1,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dsyevr'", info)));
    }
    let lwork = tmp as i32;
    let liwork = itmp;
    let mut work = vec![0.0f64; lwork.max(1) as usize];
    let mut iwork = vec![0i32; liwork.max(1) as usize];
    unsafe {
        f(
            &jobv,
            &b'A',
            &b'L',
            &n_i,
            rx.as_mut_ptr(),
            &n_i,
            &0.0,
            &0.0,
            &0,
            &0,
            &0.0,
            &mut m,
            values.as_mut_ptr(),
            zp,
            &n_i,
            isuppz.as_mut_ptr(),
            work.as_mut_ptr(),
            &lwork,
            iwork.as_mut_ptr(),
            &liwork,
            &mut info,
            1,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dsyevr'", info)));
    }
    let mut result = vec![("values", Value::reals(values))];
    if ov == 0 {
        let mut vectors = Value::reals(z);
        vectors.set_attr_raw(s::DIM, Value::ints(vec![n_i, n_i]));
        result.push(("vectors", vectors));
    }
    Ok(Value::named_list(result))
}

/// R: Lapack.c:La_rs_cmplx (zheev)
fn do_la_rs_cmplx(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let xin = arg(&args, 0);
    let (n, n2) = dims2(&xin);
    if n != n2 {
        return Err(Flow::error("'x' must be a square complex matrix"));
    }
    let ov = as_logical(&arg(&args, 1));
    if ov == NA_LOGICAL {
        return Err(Flow::error("invalid 'only.values' argument"));
    }
    let n_i = n as i32;
    let mut rx = cplx_data(&xin); // zheev overwrites it with the vectors
    let mut values = vec![0.0f64; n];
    // R allocates max(3n-2, 1) doubles (guarding n < 1).
    let rwork_len = if n >= 1 { 3 * n - 2 } else { 1 };
    let mut rwork = vec![0.0f64; rwork_len];
    let jobv: u8 = if ov != 0 { b'N' } else { b'V' };
    let f = system_lapack()?.zheev;
    let mut info = 0i32;
    let mut tmp = Z::default();
    // SAFETY: pointers refer to live allocations of the documented sizes.
    unsafe {
        f(
            &jobv,
            &b'L',
            &n_i,
            rx.as_mut_ptr(),
            &n_i,
            values.as_mut_ptr(),
            &mut tmp,
            &-1,
            rwork.as_mut_ptr(),
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zheev'", info)));
    }
    let lwork = tmp.r as i32;
    let mut work = vec![Z::default(); lwork.max(1) as usize];
    unsafe {
        f(
            &jobv,
            &b'L',
            &n_i,
            rx.as_mut_ptr(),
            &n_i,
            values.as_mut_ptr(),
            work.as_mut_ptr(),
            &lwork,
            rwork.as_mut_ptr(),
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zheev'", info)));
    }
    let mut result = vec![("values", Value::reals(values))];
    if ov == 0 {
        let mut vectors = z_to_value(rx);
        vectors.set_attr_raw(s::DIM, Value::ints(vec![n_i, n_i]));
        result.push(("vectors", vectors));
    }
    Ok(Value::named_list(result))
}

/// R: Lapack.c:unscramble
fn unscramble(imaginary: &[f64], n: usize, vecs: &[f64]) -> Value {
    let mut s = vec![Complex::new(0.0, 0.0); n * n];
    let mut j = 0;
    while j < n {
        if imaginary[j] != 0.0 {
            let j1 = j + 1;
            for i in 0..n {
                s[i + n * j].re = vecs[i + j * n];
                s[i + n * j1].re = vecs[i + j * n];
                s[i + n * j].im = vecs[i + j1 * n];
                s[i + n * j1].im = -vecs[i + j1 * n];
            }
            j = j1;
        } else {
            for i in 0..n {
                s[i + n * j].re = vecs[i + j * n];
                s[i + n * j].im = 0.0;
            }
        }
        j += 1;
    }
    let mut v = Value::cplxs(s);
    v.set_attr_raw(s::DIM, Value::ints(vec![n as i32, n as i32]));
    v
}

/// R: Lapack.c:La_rg
fn do_la_rg(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let (n, n2) = dims2(&x);
    if n != n2 {
        return Err(Flow::error("'x' must be a square numeric matrix"));
    }
    let mut xvals = reals(&x);
    let ov = as_logical(&arg(&args, 1));
    if ov == NA_LOGICAL {
        return Err(Flow::error("invalid 'only.values' argument"));
    }
    let vectors = ov == 0;
    let jobvr: u8 = if vectors { b'V' } else { b'N' };
    let n_i = n as i32;
    let mut wr = vec![0.0f64; n];
    let mut wi = vec![0.0f64; n];
    let mut right = vec![0.0f64; if vectors { n * n } else { 0 }];
    let rp: *mut f64 = if vectors { right.as_mut_ptr() } else { std::ptr::null_mut() };
    let f = system_lapack()?.dgeev;
    let mut info = 0i32;
    let mut tmp = 0.0f64;
    // SAFETY: as in La_rs; VL is NULL because JOBVL = 'N'.
    unsafe {
        f(
            &b'N',
            &jobvr,
            &n_i,
            xvals.as_mut_ptr(),
            &n_i,
            wr.as_mut_ptr(),
            wi.as_mut_ptr(),
            std::ptr::null_mut(),
            &n_i,
            rp,
            &n_i,
            &mut tmp,
            &-1,
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dgeev'", info)));
    }
    let lwork = tmp as i32;
    let mut work = vec![0.0f64; lwork.max(1) as usize];
    unsafe {
        f(
            &b'N',
            &jobvr,
            &n_i,
            xvals.as_mut_ptr(),
            &n_i,
            wr.as_mut_ptr(),
            wi.as_mut_ptr(),
            std::ptr::null_mut(),
            &n_i,
            rp,
            &n_i,
            work.as_mut_ptr(),
            &lwork,
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dgeev'", info)));
    }
    // R_AccuracyInfo.eps is DBL_EPSILON.
    let eps = f64::EPSILON;
    let complex_values = (0..n).any(|i| wi[i].abs() > 10.0 * eps * wr[i].abs());
    let values = if complex_values {
        Value::cplxs((0..n).map(|i| Complex::new(wr[i], wi[i])).collect())
    } else {
        Value::reals(wr)
    };
    let vecs = if complex_values {
        if vectors { unscramble(&wi, n, &right) } else { Value::Null }
    } else if vectors {
        let mut v = Value::reals(right);
        v.set_attr_raw(s::DIM, Value::ints(vec![n_i, n_i]));
        v
    } else {
        Value::Null
    };
    Ok(Value::named_list(vec![("values", values), ("vectors", vecs)]))
}

/// R: Lapack.c:La_rg_cmplx (zgeev)
fn do_la_rg_cmplx(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let (n, n2) = dims2(&x);
    if n != n2 {
        return Err(Flow::error("'x' must be a square numeric matrix"));
    }
    let mut xvals = cplx_data(&x);
    let ov = as_logical(&arg(&args, 1));
    if ov == NA_LOGICAL {
        return Err(Flow::error("invalid 'only.values' argument"));
    }
    let vectors = ov == 0;
    let jobvr: u8 = if vectors { b'V' } else { b'N' };
    let n_i = n as i32;
    let mut values = vec![Z::default(); n];
    let mut right = vec![Z::default(); if vectors { n * n } else { 0 }];
    let rp: *mut Z = if vectors { right.as_mut_ptr() } else { std::ptr::null_mut() };
    let mut rwork = vec![0.0f64; 2 * n];
    let f = system_lapack()?.zgeev;
    let mut info = 0i32;
    let mut tmp = Z::default();
    // SAFETY: as in La_rs; VL is NULL because JOBVL = 'N'.
    unsafe {
        f(
            &b'N',
            &jobvr,
            &n_i,
            xvals.as_mut_ptr(),
            &n_i,
            values.as_mut_ptr(),
            std::ptr::null_mut(),
            &n_i,
            rp,
            &n_i,
            &mut tmp,
            &-1,
            rwork.as_mut_ptr(),
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zgeev'", info)));
    }
    let lwork = tmp.r as i32;
    let mut work = vec![Z::default(); lwork.max(1) as usize];
    unsafe {
        f(
            &b'N',
            &jobvr,
            &n_i,
            xvals.as_mut_ptr(),
            &n_i,
            values.as_mut_ptr(),
            std::ptr::null_mut(),
            &n_i,
            rp,
            &n_i,
            work.as_mut_ptr(),
            &lwork,
            rwork.as_mut_ptr(),
            &mut info,
            1,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zgeev'", info)));
    }
    let vecs = if vectors {
        let mut v = z_to_value(right);
        v.set_attr_raw(s::DIM, Value::ints(vec![n_i, n_i]));
        v
    } else {
        Value::Null
    };
    Ok(Value::named_list(vec![("values", z_to_value(values)), ("vectors", vecs)]))
}

/// R: Lapack.c:La_svd
fn do_la_svd(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let ju = match arg(&args, 0) {
        Value::Str(t) if !t.data.is_empty() => t.data[0].as_deref().unwrap_or("").to_string(),
        _ => return Err(Flow::error("'jobu' must be a character string")),
    };
    let x = arg(&args, 1);
    let (n, p) = dims2(&x);
    let mut a = reals(&x);
    let s = arg(&args, 2);
    let u = arg(&args, 3);
    let vt = arg(&args, 4);
    if !matches!(u.get_attr(s::DIM), Some(Value::Int(_))) {
        return Err(Flow::error("non-integer dim(u)"));
    }
    if !matches!(vt.get_attr(s::DIM), Some(Value::Int(_))) {
        return Err(Flow::error("non-integer dim(vt)"));
    }
    let ldu = match u.get_attr(s::DIM) {
        Some(Value::Int(d)) if !d.data.is_empty() => d.data[0] as usize,
        _ => return Err(Flow::error("robigo: dim(u) is empty")),
    };
    let ldvt = match vt.get_attr(s::DIM) {
        Some(Value::Int(d)) if !d.data.is_empty() => d.data[0] as usize,
        _ => return Err(Flow::error("robigo: dim(vt) is empty")),
    };
    if !matches!(s, Value::Real(_)) || !matches!(u, Value::Real(_)) || !matches!(vt, Value::Real(_)) {
        return Err(Flow::error("robigo: non-real SVD output buffers are not supported"));
    }
    let k = n.min(p);
    if s.len() < k {
        return Err(Flow::error("robigo: undersized SVD output buffers are not supported"));
    }
    // dgesdd writes U and VT only for the job variants R uses ('S', 'A');
    // 'N' (and 'O', which overwrites A instead) leave them untouched.
    let ucols = match ju.as_bytes().first() {
        Some(b'A') => n,
        Some(b'S') => k,
        _ => 0,
    };
    let vtcols = match ju.as_bytes().first() {
        Some(b'A') | Some(b'S') => p,
        _ => 0,
    };
    if u.len() < ldu * ucols || vt.len() < ldvt * vtcols {
        return Err(Flow::error("robigo: undersized SVD output buffers are not supported"));
    }
    let mut d = reals(&s);
    let mut ud = reals(&u);
    let mut vd = reals(&vt);
    let mut iw = vec![0i32; 8 * k];
    let n_i = n as i32;
    let p_i = p as i32;
    let ldu_i = ldu as i32;
    let ldvt_i = ldvt as i32;
    let mut info = 0i32;
    let mut tmp = 0.0f64;
    let job = ju.as_bytes().first().copied().unwrap_or(0);
    let f = system_lapack()?.dgesdd;
    // SAFETY: as in La_rs; the output buffers were validated above.
    unsafe {
        f(
            &job,
            &n_i,
            &p_i,
            a.as_mut_ptr(),
            &n_i,
            d.as_mut_ptr(),
            ud.as_mut_ptr(),
            &ldu_i,
            vd.as_mut_ptr(),
            &ldvt_i,
            &mut tmp,
            &-1,
            iw.as_mut_ptr(),
            &mut info,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dgesdd'", info)));
    }
    let lwork = tmp as i32;
    let mut work = vec![0.0f64; lwork.max(1) as usize];
    unsafe {
        f(
            &job,
            &n_i,
            &p_i,
            a.as_mut_ptr(),
            &n_i,
            d.as_mut_ptr(),
            ud.as_mut_ptr(),
            &ldu_i,
            vd.as_mut_ptr(),
            &ldvt_i,
            work.as_mut_ptr(),
            &lwork,
            iw.as_mut_ptr(),
            &mut info,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'dgesdd'", info)));
    }
    // The buffers are returned in place with their attributes, as R does.
    let mut uout = u;
    let mut vtout = vt;
    if let Value::Real(v) = &mut uout {
        std::rc::Rc::make_mut(v).data = ud;
    }
    if let Value::Real(v) = &mut vtout {
        std::rc::Rc::make_mut(v).data = vd;
    }
    Ok(Value::named_list(vec![("d", Value::reals(d)), ("u", uout), ("vt", vtout)]))
}

/// R: Lapack.c:La_svd_cmplx (zgesdd)
fn do_la_svd_cmplx(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let jz = match arg(&args, 0) {
        Value::Str(t) if !t.data.is_empty() => t.data[0].as_deref().unwrap_or("").to_string(),
        _ => return Err(Flow::error("'jobu' must be a character string")),
    };
    let x = arg(&args, 1);
    let (n, p) = dims2(&x);
    // The underlying LAPACK, specifically ZLARF, does not work with long
    // arrays.
    if (n as f64) * (p as f64) > i32::MAX as f64 {
        return Err(Flow::error("matrices of 2^31 or more elements are not supported"));
    }
    let s = arg(&args, 2);
    let u = arg(&args, 3);
    let v = arg(&args, 4);
    let ldu = match u.get_attr(s::DIM) {
        Some(Value::Int(d)) if !d.data.is_empty() => d.data[0] as usize,
        _ => return Err(Flow::error("non-integer dims")),
    };
    let ldv = match v.get_attr(s::DIM) {
        Some(Value::Int(d)) if !d.data.is_empty() => d.data[0] as usize,
        _ => return Err(Flow::error("non-integer dims")),
    };
    if !matches!(s, Value::Real(_)) || !matches!(u, Value::Cplx(_)) || !matches!(v, Value::Cplx(_)) {
        return Err(Flow::error("robigo: complex SVD needs the real/complex buffers R allocates"));
    }
    let k = n.min(p);
    if s.len() < k {
        return Err(Flow::error("robigo: undersized SVD output buffers are not supported"));
    }
    let job = jz.as_bytes().first().copied().unwrap_or(0);
    // zgesdd writes U/V for the job variants R uses; 'N' leaves them alone.
    let ucols = match job {
        b'A' => n,
        b'S' => k,
        _ => 0,
    };
    let vcols = match job {
        b'A' | b'S' => p,
        _ => 0,
    };
    if u.len() < ldu * ucols || v.len() < ldv * vcols {
        return Err(Flow::error("robigo: undersized SVD output buffers are not supported"));
    }
    let mut xvals = cplx_data(&x);
    let mut d = reals(&s);
    let mut ud = cplx_data(&u);
    let mut vd = cplx_data(&v);
    let mut iw = vec![0i32; 8 * k];
    let mn0 = k;
    let mn1 = n.max(p);
    let lrwork = if job != b'N' {
        let f1 = 5 * mn1 + 7;
        let f2 = 2 * mn1 + 2 * mn0 + 1;
        // 7 replaces 5: bug 111 in netlib's LAPACK errata.
        f1.max(f2) * mn0
    } else {
        7 * mn0
    };
    let mut rwork = vec![0.0f64; lrwork.max(1)];
    let n_i = n as i32;
    let p_i = p as i32;
    let ldu_i = ldu as i32;
    let ldv_i = ldv as i32;
    let f = system_lapack()?.zgesdd;
    let mut info = 0i32;
    let mut tmp = Z::default();
    // SAFETY: output buffers validated above; sizes as documented.
    unsafe {
        f(
            &job,
            &n_i,
            &p_i,
            xvals.as_mut_ptr(),
            &n_i,
            d.as_mut_ptr(),
            ud.as_mut_ptr(),
            &ldu_i,
            vd.as_mut_ptr(),
            &ldv_i,
            &mut tmp,
            &-1,
            rwork.as_mut_ptr(),
            iw.as_mut_ptr(),
            &mut info,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zgesdd'", info)));
    }
    let lwork = tmp.r as i32;
    let mut work = vec![Z::default(); lwork.max(1) as usize];
    unsafe {
        f(
            &job,
            &n_i,
            &p_i,
            xvals.as_mut_ptr(),
            &n_i,
            d.as_mut_ptr(),
            ud.as_mut_ptr(),
            &ldu_i,
            vd.as_mut_ptr(),
            &ldv_i,
            work.as_mut_ptr(),
            &lwork,
            rwork.as_mut_ptr(),
            iw.as_mut_ptr(),
            &mut info,
            1,
        );
    }
    if info != 0 {
        return Err(Flow::error(format!("error code {} from Lapack routine 'zgesdd'", info)));
    }
    let mut uout = u;
    let mut vout = v;
    if let Value::Cplx(c) = &mut uout {
        std::rc::Rc::make_mut(c).data = ud.into_iter().map(|z| Complex::new(z.r, z.i)).collect();
    }
    if let Value::Cplx(c) = &mut vout {
        std::rc::Rc::make_mut(c).data = vd.into_iter().map(|z| Complex::new(z.r, z.i)).collect();
    }
    Ok(Value::named_list(vec![("d", Value::reals(d)), ("u", uout), ("vt", vout)]))
}

/// R: Lapack.c:La_solve
fn do_la_solve(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    let bin = arg(&args, 1);
    let tol = as_real(&arg(&args, 2));
    if !is_numeric_matrix(&a) {
        return Err(Flow::error("'a' must be a numeric matrix"));
    }
    let (n, n2) = dims2(&a);
    if n == 0 {
        return Err(Flow::error("'a' is 0-diml"));
    }
    if n2 != n {
        return Err(Flow::error(format!("'a' ({} x {}) must be square", n, n2)));
    }
    let adn_present = a.get_attr(s::DIMNAMES).is_some();
    let b_is_matrix = matches!(bin.get_attr(s::DIM), Some(d) if d.len() == 2);
    let p: usize;
    // Attributes of the result, set once the solution is computed.
    let mut out_dimnames = Value::Null;
    let mut out_names = Value::Null;
    let bvals = reals(&bin);
    if b_is_matrix {
        let (p2, pp) = dims2(&bin);
        p = pp;
        if p == 0 {
            return Err(Flow::error("no right-hand side in 'b'"));
        }
        if p2 != n {
            return Err(Flow::error(format!("'b' ({} x {}) must be compatible with 'a' ({} x {})", p2, p, n, n)));
        }
        let bindn_present = bin.get_attr(s::DIMNAMES).is_some();
        if adn_present || bindn_present {
            let r = if adn_present { dimnames_elt(&a, 1) } else { Value::Null };
            let c = if bindn_present { dimnames_elt(&bin, 1) } else { Value::Null };
            if !r.is_null() || !c.is_null() {
                out_dimnames = Value::list(vec![r, c]);
            }
        }
    } else {
        p = 1;
        if bin.len() != n {
            return Err(Flow::error(format!(
                "'b' ({} x {}) must be compatible with 'a' ({} x {})",
                bin.len(),
                p,
                n,
                n
            )));
        }
        if adn_present {
            out_names = dimnames_elt(&a, 1);
        }
    }
    let mut b: Vec<f64> = bvals[..n * p].to_vec();
    let aorig = reals(&a);
    let mut avals = aorig.clone();
    let mut ipiv = vec![0usize; n];
    let info = dgetrf(&mut avals, n, n, n, &mut ipiv);
    if info > 0 {
        return Err(Flow::error(format!("Lapack routine dgesv: system is exactly singular: U[{},{}] = 0", info, info)));
    }
    dgetrs(&avals, n, &ipiv, &mut b, p);
    let ok = avals.iter().all(|v| v.is_finite());
    if ok && tol > 0.0 {
        let anorm = dlange_one(&aorig, n, n);
        let rcond = dgecon(true, n, &avals, anorm);
        if rcond < tol {
            return Err(Flow::error(format!(
                "system is computationally singular: reciprocal condition number = {}",
                fmt_g(rcond)
            )));
        }
    }
    let mut out = Value::reals(b);
    if b_is_matrix {
        out.set_attr_raw(s::DIM, Value::ints(vec![n as i32, p as i32]));
        out.set_attr_raw(s::DIMNAMES, out_dimnames);
    } else {
        out.set_attr_raw(s::NAMES, out_names);
    }
    Ok(out)
}

/// R: Lapack.c:La_solve_cmplx (zgesv, then zlange/zgecon for the
/// reciprocal condition number)
fn do_la_solve_cmplx(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    let bin = arg(&args, 1);
    let tolin = arg(&args, 2);
    if !matches!(a.get_attr(s::DIM), Some(d) if d.len() == 2) {
        return Err(Flow::error("'a' must be a complex matrix"));
    }
    let (n, n2) = dims2(&a);
    if n == 0 {
        return Err(Flow::error("'a' is 0-diml"));
    }
    if n2 != n {
        return Err(Flow::error(format!("'a' ({} x {}) must be square", n, n2)));
    }
    let adn_present = a.get_attr(s::DIMNAMES).is_some();
    let b_is_matrix = matches!(bin.get_attr(s::DIM), Some(d) if d.len() == 2);
    let p: usize;
    // Attributes of the result, set once the solution is computed.
    let mut out_dimnames = Value::Null;
    let mut out_names = Value::Null;
    if b_is_matrix {
        let (p2, pp) = dims2(&bin);
        p = pp;
        if p == 0 {
            return Err(Flow::error("no right-hand side in 'b'"));
        }
        if p2 != n {
            return Err(Flow::error(format!("'b' ({} x {}) must be compatible with 'a' ({} x {})", p2, p, n, n)));
        }
        let bindn_present = bin.get_attr(s::DIMNAMES).is_some();
        if adn_present || bindn_present {
            let r = if adn_present { dimnames_elt(&a, 1) } else { Value::Null };
            let c = if bindn_present { dimnames_elt(&bin, 1) } else { Value::Null };
            // The complex case sets dimnames only when both parts exist.
            if !r.is_null() && !c.is_null() {
                out_dimnames = Value::list(vec![r, c]);
            }
        }
    } else {
        p = 1;
        if bin.len() != n {
            return Err(Flow::error(format!(
                "'b' ({} x {}) must be compatible with 'a' ({} x {})",
                bin.len(),
                p,
                n,
                n
            )));
        }
        if adn_present {
            out_names = dimnames_elt(&a, 1);
        }
    }
    let bvals = cplx_data(&bin);
    let mut b: Vec<Z> = bvals[..n * p].to_vec();
    let aorig = cplx_data(&a);
    let mut avals = aorig.clone();
    let mut ipiv = vec![0i32; n];
    let n_i = n as i32;
    let p_i = p as i32;
    let mut info = 0i32;
    let f = system_lapack()?.zgesv;
    // SAFETY: pointers refer to live allocations of the documented sizes.
    unsafe {
        f(&n_i, &p_i, avals.as_mut_ptr(), &n_i, ipiv.as_mut_ptr(), b.as_mut_ptr(), &n_i, &mut info);
    }
    if info < 0 {
        return Err(Flow::error(format!("argument {} of Lapack routine {} had invalid value", -info, "zgesv")));
    }
    if info > 0 {
        return Err(Flow::error(format!(
            "Lapack routine {}: system is exactly singular: U[{},{}] = 0",
            "zgesv", info, info
        )));
    }
    let ok = avals.iter().all(|z| z.r.is_finite() && z.i.is_finite());
    let tol = as_real(&tolin);
    if ok && tol > 0.0 {
        let lib = system_lapack()?;
        // SAFETY: zlange with NORM='1' does not reference WORK.
        let anorm = unsafe { (lib.zlange)(&b'1', &n_i, &n_i, aorig.as_ptr(), &n_i, std::ptr::null_mut(), 1) };
        let mut work = vec![Z::default(); 2 * n];
        let mut rwork = vec![0.0f64; 2 * n];
        let mut rcond = 0.0f64;
        let mut cinfo = 0i32;
        // SAFETY: as documented; A here is the LU factorization in avals.
        unsafe {
            (lib.zgecon)(
                &b'1',
                &n_i,
                avals.as_ptr(),
                &n_i,
                &anorm,
                &mut rcond,
                work.as_mut_ptr(),
                rwork.as_mut_ptr(),
                &mut cinfo,
                1,
            );
        }
        if rcond < tol {
            return Err(Flow::error(format!(
                "system is computationally singular: reciprocal condition number = {}",
                fmt_g(rcond)
            )));
        }
    }
    let mut out = z_to_value(b);
    if b_is_matrix {
        out.set_attr_raw(s::DIM, Value::ints(vec![n_i, p_i]));
        out.set_attr_raw(s::DIMNAMES, out_dimnames);
    } else {
        out.set_attr_raw(s::NAMES, out_names);
    }
    Ok(out)
}

/// R: Lapack.c:det_ge_real
fn do_det_ge_real(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let ain = arg(&args, 0);
    let use_log = as_logical(&arg(&args, 1));
    if !matches!(ain.get_attr(s::DIM), Some(d) if d.len() == 2) {
        return Err(Flow::error("'a' must be a numeric matrix"));
    }
    if use_log == NA_LOGICAL {
        return Err(Flow::error("argument 'logarithm' must be logical"));
    }
    let mut a = reals(&ain);
    let (n, n2) = dims2(&ain);
    if n2 != n {
        return Err(Flow::error("'a' must be a square matrix"));
    }
    let mut jpvt = vec![0usize; n];
    let info = dgetrf(&mut a, n, n, n, &mut jpvt);
    let mut sign = 1;
    let modulus;
    if info > 0 {
        modulus = if use_log != 0 { f64::NEG_INFINITY } else { 0.0 };
    } else {
        for (k, &p) in jpvt.iter().enumerate() {
            if p != k {
                sign = -sign;
            }
        }
        if use_log != 0 {
            let mut m = 0.0;
            for k in 0..n {
                let dii = a[k * (n + 1)];
                m += (if dii < 0.0 { -dii } else { dii }).ln();
                if dii < 0.0 {
                    sign = -sign;
                }
            }
            modulus = m;
        } else {
            let mut m = 1.0;
            for k in 0..n {
                m *= a[k * (n + 1)];
            }
            if m < 0.0 {
                m = -m;
                sign = -sign;
            }
            modulus = m;
        }
    }
    let mut md = Value::real(modulus);
    md.set_attr_raw(Sym::new("logarithm"), Value::lgl(use_log != 0));
    let mut val = Value::named_list(vec![("modulus", md), ("sign", Value::int(sign))]);
    val.set_attr_raw(s::CLASS, Value::str("det"));
    Ok(val)
}

/// R: Lapack.c:La_chol
fn do_la_chol(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    if !matches!(a.get_attr(s::DIM), Some(d) if d.len() == 2) {
        return Err(Flow::error("'a' must be a numeric matrix"));
    }
    let (m, n) = dims2(&a);
    if m != n {
        return Err(Flow::error("'a' must be a square matrix"));
    }
    if m == 0 {
        return Err(Flow::error("'a' must have dims > 0"));
    }
    let mut ans = coerce_quiet(&a, VType::Real);
    let mut data = reals(&ans);
    for j in 0..n {
        for i in j + 1..n {
            data[i + n * j] = 0.0;
        }
    }
    let piv = as_integer(&arg(&args, 1));
    if piv != 0 && piv != 1 {
        return Err(Flow::error("invalid 'pivot' value"));
    }
    if piv == 0 {
        let info = dpotrf_upper(&mut data, n);
        if info != 0 {
            if info > 0 {
                return Err(Flow::error(format!("the leading minor of order {} is not positive", info)));
            }
            return Err(Flow::error(format!(
                "argument {} of Lapack routine {} had invalid value",
                -(info as i64),
                "dpotrf"
            )));
        }
    } else {
        // R: Lapack.c:La_chol's dpstrf branch.
        let tol = as_real(&arg(&args, 2));
        let mut piv = vec![0i32; n];
        let mut work = vec![0.0; 2 * n];
        let mut rank = 0i32;
        let mut info = 0i32;
        let f = system_lapack()?.dpstrf;
        // SAFETY: pointers refer to live allocations of the documented sizes.
        unsafe {
            f(
                &b'U',
                &(n as i32),
                data.as_mut_ptr(),
                &(n as i32),
                piv.as_mut_ptr(),
                &mut rank,
                &tol,
                work.as_mut_ptr(),
                &mut info,
                1,
            );
        }
        if info != 0 {
            if info > 0 {
                i.warning("the matrix is either rank-deficient or not positive definite")?;
            } else {
                return Err(Flow::error(format!(
                    "argument {} of Lapack routine {} had invalid value",
                    -info, "dpstrf"
                )));
            }
        }
        if let Value::Real(r) = &mut ans {
            std::rc::Rc::make_mut(r).data = data;
        }
        ans.set_attr_raw(Sym::new("pivot"), Value::ints(piv.clone()));
        ans.set_attr_raw(Sym::new("rank"), Value::int(rank));
        // Pivot the column names when the result has any.
        if let Some(Value::List(dn)) = ans.get_attr(s::DIMNAMES) {
            if let Some(Value::Str(cn)) = dn.data.get(1) {
                let permuted: Vec<RStr> = piv.iter().map(|&p| cn.data[(p - 1) as usize].clone()).collect();
                let mut dn2 = dn.data.clone();
                dn2[1] = Value::Str(std::rc::Rc::new(Vector::new(permuted)));
                ans.set_attr_raw(s::DIMNAMES, Value::list(dn2));
            }
        }
        return Ok(ans);
    }
    if let Value::Real(r) = &mut ans {
        std::rc::Rc::make_mut(r).data = data;
    }
    Ok(ans)
}

/// R: Lapack.c:La_chol2inv
fn do_la_chol2inv(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    let sz = as_integer(&arg(&args, 1));
    if sz == NA_INTEGER || sz < 1 {
        return Err(Flow::error("'size' argument must be a positive integer"));
    }
    let sz = sz as usize;
    let is_mat = matches!(a.get_attr(s::DIM), Some(d) if d.len() == 2);
    let (m, n) = if sz == 1 && !is_mat && matches!(a, Value::Real(_)) {
        (1, 1)
    } else if is_mat {
        dims2(&a)
    } else {
        return Err(Flow::error("'a' must be a numeric matrix"));
    };
    if sz > n {
        return Err(Flow::error(format!("'size' cannot exceed ncol(x) = {}", n)));
    }
    if sz > m {
        return Err(Flow::error(format!("'size' cannot exceed nrow(x) = {}", m)));
    }
    let av = reals(&a);
    let mut ans = vec![0.0; sz * sz];
    for j in 0..sz {
        for i in 0..=j {
            ans[i + j * sz] = av[i + j * m];
        }
    }
    let info = dpotri_upper(&mut ans, sz);
    if info > 0 {
        return Err(Flow::error(format!("element ({}, {}) is zero, so the inverse cannot be computed", info, info)));
    }
    for j in 0..sz {
        for i in j + 1..sz {
            ans[i + j * sz] = ans[j + i * sz];
        }
    }
    let mut v = Value::reals(ans);
    v = set_attrib(v, s::DIM, Value::ints(vec![sz as i32, sz as i32]))?;
    Ok(v)
}

/// R: Lapack.c:La_dlange
fn do_la_dlange(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    if !is_numeric_matrix(&a) {
        return Err(Flow::error("'A' must be a numeric matrix"));
    }
    let typ = match arg(&args, 1) {
        Value::Str(t) if !t.data.is_empty() => t.data[0].as_deref().unwrap_or("").to_string(),
        _ => return Err(Flow::error("'type' must be a character string")),
    };
    let (m, n) = dims2(&a);
    let x = reals(&a);
    let r = match typ.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('O') | Some('1') => dlange_one(&x, m, n),
        Some('I') => dlange_inf(&x, m, n),
        Some('M') => x.iter().fold(0.0f64, |acc, v| if acc < v.abs() || v.is_nan() { v.abs() } else { acc }),
        Some('F') | Some('E') => {
            // DLASSQ-based scaled sum of squares
            let mut scale = 0.0f64;
            let mut sumsq = 1.0f64;
            for &v in &x {
                if v != 0.0 {
                    let absxi = v.abs();
                    if scale < absxi {
                        sumsq = 1.0 + sumsq * (scale / absxi) * (scale / absxi);
                        scale = absxi;
                    } else {
                        sumsq += (absxi / scale) * (absxi / scale);
                    }
                }
            }
            scale * sumsq.sqrt()
        }
        _ => return Err(Flow::error(format!("argument type[1]='{}' must be one of 'M','1','O','I','F' or 'E'", typ))),
    };
    Ok(Value::real(r))
}

/// R: Lapack.c:La_dgecon
fn do_la_dgecon(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let a = arg(&args, 0);
    if !is_numeric_matrix(&a) {
        return Err(Flow::error("'A' must be a numeric matrix"));
    }
    let norm = match arg(&args, 1) {
        Value::Str(t) if !t.data.is_empty() => t.data[0].as_deref().unwrap_or("").to_string(),
        _ => return Err(Flow::error("'norm' must be a character string")),
    };
    let (m, n) = dims2(&a);
    if m != n {
        return Err(Flow::error("'A' must be a square matrix"));
    }
    if n == 0 {
        return Err(Flow::error("'A' must be a square matrix"));
    }
    let onenrm = norm.starts_with('O') || norm.starts_with('1');
    if !onenrm && !norm.starts_with('I') {
        return Err(Flow::error(format!("'norm' must be one of 'O', 'I'")));
    }
    let x = reals(&a);
    let anorm = if onenrm { dlange_one(&x, n, n) } else { dlange_inf(&x, n, n) };
    let mut lu = x;
    let mut ipiv = vec![0usize; n];
    let info = dgetrf(&mut lu, n, n, n, &mut ipiv);
    if info > 0 {
        return Ok(Value::real(0.0));
    }
    Ok(Value::real(dgecon(onenrm, n, &lu, anorm)))
}

fn do_la_version(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: Lapack.c case 1000: ask the bound LAPACK's ilaver.
    let f = system_lapack()?.ilaver;
    let (mut major, mut minor, mut patch) = (0i32, 0i32, 0i32);
    // SAFETY: three writable i32 locals.
    unsafe { f(&mut major, &mut minor, &mut patch) };
    Ok(Value::str(&format!("{}.{}.{}", major, minor, patch)))
}

fn do_la_library(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: Lapack.c case 1001: dladdr the ilaver we call, realpath its
    // object. (R additionally reports the flexiblas backend when the
    // library is flexiblas; no supported platform here uses it.)
    let Some(f) = ILAVER_PTR.get() else {
        // No bound library: R also answers "" when it cannot tell.
        return Ok(Value::str(""));
    };
    // SAFETY: dladdr reads the pointer and writes `info`; realpath writes
    // at most PATH_MAX+1 bytes into `buf`.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(f.0, &mut info) != 0 {
            let mut buf = [0i8; (libc::PATH_MAX as usize) + 1];
            let res = libc::realpath(info.dli_fname, buf.as_mut_ptr());
            if !res.is_null() {
                let path = std::ffi::CStr::from_ptr(res).to_string_lossy().into_owned();
                return Ok(Value::str(&path));
            }
        }
    }
    Ok(Value::str(""))
}
