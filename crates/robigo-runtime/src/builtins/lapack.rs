//! LAPACK-based linear algebra internals (GNU R's modules/lapack/Lapack.c).
//!
//! The LAPACK and reference BLAS routines these need are ported from the
//! Fortran sources R ships (`src/modules/lapack/dlapack.f`,
//! `src/extra/blas/blas.f`), keeping their operation order so results
//! agree with R linked against the reference libraries. Matrices are
//! column-major slices with a leading dimension, as in Fortran.

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
    internal("La_solve", do_la_solve, 3),
    internal("La_solve_cmplx", do_la_unimplemented, 2),
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

fn do_la_unimplemented(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Err(Flow::error("robigo: complex linear algebra (La_solve_cmplx) is not implemented yet"))
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
fn do_la_chol(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
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
    if piv == 1 {
        return Err(Flow::error("robigo: chol(pivot = TRUE) is not implemented yet"));
    }
    let info = dpotrf_upper(&mut data, n);
    if info > 0 {
        return Err(Flow::error(format!("the leading minor of order {} is not positive", info)));
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
    // The reference LAPACK version whose routines are ported here.
    Ok(Value::str("3.12.0"))
}

fn do_la_library(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R returns "" when the LAPACK library is not known; robigo's routines are
    // built in.
    Ok(Value::str(""))
}
