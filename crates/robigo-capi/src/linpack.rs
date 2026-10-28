//! GNU R's LINPACK routines (src/appl/*.f), with the Fortran calling
//! convention libR exports them with (R_ext/Linpack.h), and the reference
//! BLAS level-1 routines the C code of src/appl calls. The BLAS symbols
//! are not exported from the executable (packages link their own BLAS);
//! they serve the appl objects built into robigo (build.rs).

use std::ffi::c_int;

use robigo_runtime::native::linpack as lp;

// The functions of GNU R's src/appl C files (appl/, compiled by build.rs
// into a static library): referenced here so that the linker keeps them
// in the executable, which exports them.
unsafe extern "C" {
    fn vmmin();
    fn nmmin();
    fn cgmin();
    fn samin();
    fn lbfgsb();
    fn optif9();
    fn fdhess();
    fn Rdqags();
    fn Rdqagi();
}

#[used]
static APPL: [unsafe extern "C" fn(); 9] = [vmmin, nmmin, cgmin, samin, lbfgsb, optif9, fdhess, Rdqags, Rdqagi];

unsafe fn sl<'a>(p: *const f64, n: usize) -> &'a [f64] {
    if n == 0 { &[] } else { std::slice::from_raw_parts(p, n) }
}

unsafe fn sl_mut<'a>(p: *mut f64, n: usize) -> &'a mut [f64] {
    if n == 0 { &mut [] } else { std::slice::from_raw_parts_mut(p, n) }
}

fn us(n: c_int) -> usize {
    n.max(0) as usize
}

// ---- reference BLAS (level 1), any increments -------------------------

/// The elements `x(1), x(1+inc), ...` (reference BLAS: a negative
/// increment starts at the far end).
unsafe fn strided(x: *const f64, n: usize, inc: c_int) -> Vec<f64> {
    let step = inc.unsigned_abs() as usize;
    let start = if inc < 0 { (n - 1) * step } else { 0 };
    (0..n).map(|i| if inc < 0 { *x.add(start - i * step) } else { *x.add(i * step) }).collect()
}

unsafe fn idx(n: usize, inc: c_int, i: usize) -> isize {
    let step = inc as isize;
    if inc < 0 { (n as isize - 1 - i as isize) * -step } else { i as isize * step }
}

// R: BLAS ddot (blas.f)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ddot_(
    n: *const c_int,
    dx: *const f64,
    incx: *const c_int,
    dy: *const f64,
    incy: *const c_int,
) -> f64 {
    let n = us(*n);
    if n == 0 {
        return 0.0;
    }
    if *incx == 1 && *incy == 1 {
        return lp::ddot(n, sl(dx, n), sl(dy, n));
    }
    let mut t = 0.0;
    for i in 0..n {
        t += *dx.offset(idx(n, *incx, i)) * *dy.offset(idx(n, *incy, i));
    }
    t
}

// R: BLAS daxpy
#[unsafe(no_mangle)]
pub unsafe extern "C" fn daxpy_(
    n: *const c_int,
    da: *const f64,
    dx: *const f64,
    incx: *const c_int,
    dy: *mut f64,
    incy: *const c_int,
) {
    let n = us(*n);
    if n == 0 || *da == 0.0 {
        return;
    }
    if *incx == 1 && *incy == 1 {
        let x = sl(dx, n).to_vec();
        lp::daxpy(n, *da, &x, sl_mut(dy, n));
        return;
    }
    for i in 0..n {
        let yi = dy.offset(idx(n, *incy, i));
        *yi += *da * *dx.offset(idx(n, *incx, i));
    }
}

// R: BLAS dcopy
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dcopy_(n: *const c_int, dx: *const f64, incx: *const c_int, dy: *mut f64, incy: *const c_int) {
    let n = us(*n);
    for i in 0..n {
        *dy.offset(idx(n, *incy, i)) = *dx.offset(idx(n, *incx, i));
    }
}

// R: BLAS dscal
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dscal_(n: *const c_int, da: *const f64, dx: *mut f64, incx: *const c_int) {
    let n = us(*n);
    if n == 0 || *incx <= 0 {
        return;
    }
    if *incx == 1 {
        lp::dscal(n, *da, sl_mut(dx, n));
        return;
    }
    for i in 0..n {
        *dx.add(i * *incx as usize) *= *da;
    }
}

// R: BLAS dnrm2 (dnrm2.f90)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dnrm2_(n: *const c_int, x: *const f64, incx: *const c_int) -> f64 {
    let n = us(*n);
    if n == 0 {
        return 0.0;
    }
    let v = strided(x, n, *incx);
    lp::dnrm2(n, &v)
}

// R: BLAS dswap
unsafe fn dswap(n: usize, x: *mut f64, y: *mut f64) {
    for i in 0..n {
        std::ptr::swap(x.add(i), y.add(i));
    }
}

// ---- LINPACK -----------------------------------------------------------

// R: appl/dqrdc2.f
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrdc2_(
    x: *mut f64,
    ldx: *const c_int,
    n: *const c_int,
    p: *const c_int,
    tol: *const f64,
    k: *mut c_int,
    qraux: *mut f64,
    jpvt: *mut c_int,
    work: *mut f64,
) {
    let (ldx, n, p) = (us(*ldx), us(*n), us(*p));
    let x = sl_mut(x, ldx * p);
    let jp = if p == 0 { &mut [][..] } else { std::slice::from_raw_parts_mut(jpvt, p) };
    *k = lp::dqrdc2(x, ldx, n, p, *tol, sl_mut(qraux, p), jp, sl_mut(work, 2 * p)) as c_int;
}

// R: appl/dqrsl.f (y is read before any output is written, so outputs
// may alias it as R's callers do)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrsl_(
    x: *mut f64,
    ldx: *const c_int,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *const f64,
    qy: *mut f64,
    qty: *mut f64,
    b: *mut f64,
    rsd: *mut f64,
    xb: *mut f64,
    job: *const c_int,
    info: *mut c_int,
) {
    let (ldx, n, k, job) = (us(*ldx), us(*n), us(*k), *job);
    let yv = sl(y, n).to_vec();
    let x = sl_mut(x, ldx * k);
    let q = sl(qraux, k);
    let mut o = lp::Qrsl::default();
    if job / 10000 != 0 {
        o.qy = Some(sl_mut(qy, n));
    }
    if job % 10000 != 0 {
        o.qty = Some(sl_mut(qty, n));
    }
    if (job % 1000) / 100 != 0 {
        o.b = Some(sl_mut(b, k));
    }
    if (job % 100) / 10 != 0 {
        o.rsd = Some(sl_mut(rsd, n));
    }
    if job % 10 != 0 {
        o.xb = Some(sl_mut(xb, n));
    }
    *info = lp::dqrsl(x, ldx, n, k, q, &yv, &mut o, job);
}

// R: appl/dqrls.f
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrls_(
    x: *mut f64,
    n: *const c_int,
    p: *const c_int,
    y: *const f64,
    ny: *const c_int,
    tol: *const f64,
    b: *mut f64,
    rsd: *mut f64,
    qty: *mut f64,
    k: *mut c_int,
    jpvt: *mut c_int,
    qraux: *mut f64,
    work: *mut f64,
) {
    let (n, p, ny) = (us(*n), us(*p), us(*ny));
    let yv = sl(y, n * ny).to_vec();
    let jp = if p == 0 { &mut [][..] } else { std::slice::from_raw_parts_mut(jpvt, p) };
    *k = lp::dqrls(
        sl_mut(x, n * p),
        n,
        p,
        &yv,
        ny,
        *tol,
        sl_mut(b, p * ny),
        sl_mut(rsd, n * ny),
        sl_mut(qty, n * ny),
        jp,
        sl_mut(qraux, p),
        sl_mut(work, 2 * p),
    ) as c_int;
}

unsafe fn qrutl(
    what: lp::Util,
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    out: *mut f64,
    outlen: usize,
) -> c_int {
    let (n, k, ny) = (us(*n), us(*k), us(*ny));
    let mut ycopy = sl(y, n * ny).to_vec();
    let mut o = vec![0.0; outlen];
    let info = lp::dqrutl(what, sl_mut(x, n * k), n, k, sl(qraux, k), &mut ycopy, ny, &mut o);
    // y holds Q'y afterwards for the qty-based drivers, as in R
    sl_mut(y, n * ny).copy_from_slice(&ycopy);
    sl_mut(out, outlen).copy_from_slice(&o);
    info
}

// R: appl/dqrutl.f
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrcf_(
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    b: *mut f64,
    info: *mut c_int,
) {
    *info = qrutl(lp::Util::Coef, x, n, k, qraux, y, ny, b, us(*k) * us(*ny));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrqty_(
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    qty: *mut f64,
) {
    qrutl(lp::Util::Qty, x, n, k, qraux, y, ny, qty, us(*n) * us(*ny));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrqy_(
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    qy: *mut f64,
) {
    qrutl(lp::Util::Qy, x, n, k, qraux, y, ny, qy, us(*n) * us(*ny));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrrsd_(
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    rsd: *mut f64,
) {
    qrutl(lp::Util::Rsd, x, n, k, qraux, y, ny, rsd, us(*n) * us(*ny));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrxb_(
    x: *mut f64,
    n: *const c_int,
    k: *const c_int,
    qraux: *const f64,
    y: *mut f64,
    ny: *const c_int,
    xb: *mut f64,
) {
    qrutl(lp::Util::Xb, x, n, k, qraux, y, ny, xb, us(*n) * us(*ny));
}

// R: appl/dqrdc.f (LINPACK's Householder QR with optional pivoting)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dqrdc_(
    x: *mut f64,
    ldx: *const c_int,
    n: *const c_int,
    p: *const c_int,
    qraux: *mut f64,
    jpvt: *mut c_int,
    work: *mut f64,
    job: *const c_int,
) {
    let (ldx, n, p) = (us(*ldx), *n, *p);
    // 1-based Fortran indexing
    let xa = |i: c_int, j: c_int| x.add((i - 1) as usize + (j - 1) as usize * ldx);
    let col = |j: c_int| x.add((j - 1) as usize * ldx);
    let q = |j: c_int| qraux.add((j - 1) as usize);
    let w = |j: c_int| work.add((j - 1) as usize);
    let jv = |j: c_int| jpvt.add((j - 1) as usize);
    let one: c_int = 1;
    let mut pl = 1;
    let mut pu = 0;
    if *job != 0 {
        for j in 1..=p {
            let swapj = *jv(j) > 0;
            let negj = *jv(j) < 0;
            *jv(j) = j;
            if negj {
                *jv(j) = -j;
            }
            if swapj {
                if j != pl {
                    dswap(n.max(0) as usize, col(pl), col(j));
                }
                *jv(j) = *jv(pl);
                *jv(pl) = j;
                pl += 1;
            }
        }
        pu = p;
        for jj in 1..=p {
            let j = p - jj + 1;
            if *jv(j) < 0 {
                *jv(j) = -*jv(j);
                if j != pu {
                    dswap(n.max(0) as usize, col(pu), col(j));
                    let jp = *jv(pu);
                    *jv(pu) = *jv(j);
                    *jv(j) = jp;
                }
                pu -= 1;
            }
        }
    }
    if pu >= pl {
        for j in pl..=pu {
            *q(j) = dnrm2_(&n, col(j), &one);
            *w(j) = *q(j);
        }
    }
    let lup = n.min(p);
    for l in 1..=lup {
        if !(l < pl || l >= pu) {
            let mut maxnrm = 0.0;
            let mut maxj = l;
            for j in l..=pu {
                if *q(j) > maxnrm {
                    maxnrm = *q(j);
                    maxj = j;
                }
            }
            if maxj != l {
                dswap(n.max(0) as usize, col(l), col(maxj));
                *q(maxj) = *q(l);
                *w(maxj) = *w(l);
                let jp = *jv(maxj);
                *jv(maxj) = *jv(l);
                *jv(l) = jp;
            }
        }
        *q(l) = 0.0;
        if l == n {
            continue;
        }
        let m = n - l + 1;
        let mut nrmxl = dnrm2_(&m, xa(l, l), &one);
        if nrmxl == 0.0 {
            continue;
        }
        if *xa(l, l) != 0.0 {
            nrmxl = if *xa(l, l) >= 0.0 { nrmxl.abs() } else { -nrmxl.abs() };
        }
        let s = 1.0 / nrmxl;
        dscal_(&m, &s, xa(l, l), &one);
        *xa(l, l) += 1.0;
        let lp1 = l + 1;
        if p >= lp1 {
            for j in lp1..=p {
                let t = -ddot_(&m, xa(l, l), &one, xa(l, j), &one) / *xa(l, l);
                daxpy_(&m, &t, xa(l, l), &one, xa(l, j), &one);
                if j < pl || j > pu || *q(j) == 0.0 {
                    continue;
                }
                let r = (*xa(l, j)).abs() / *q(j);
                let mut tt = 1.0 - r * r;
                tt = tt.max(0.0);
                let t = tt;
                let r2 = *q(j) / *w(j);
                tt = 1.0 + 0.05 * tt * (r2 * r2);
                if tt != 1.0 {
                    *q(j) *= t.sqrt();
                } else {
                    let m1 = n - l;
                    *q(j) = dnrm2_(&m1, xa(l + 1, j), &one);
                    *w(j) = *q(j);
                }
            }
        }
        *q(l) = *xa(l, l);
        *xa(l, l) = -nrmxl;
    }
}

// R: appl/dpofa.f (with R's tolerance for positive definiteness)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dpofa_(a: *mut f64, lda: *const c_int, n: *const c_int, info: *mut c_int) {
    let lda = us(*lda);
    let at = |i: c_int, j: c_int| a.add((i - 1) as usize + (j - 1) as usize * lda);
    let one: c_int = 1;
    let eps = 1e-14;
    for j in 1..=*n {
        *info = j;
        let mut s = 0.0;
        let jm1 = j - 1;
        for k in 1..=jm1 {
            let km1 = k - 1;
            let mut t = *at(k, j) - ddot_(&km1, at(1, k), &one, at(1, j), &one);
            t /= *at(k, k);
            *at(k, j) = t;
            s += t * t;
        }
        s = *at(j, j) - s;
        if s <= eps * (*at(j, j)).abs() {
            return;
        }
        *at(j, j) = s.sqrt();
    }
    *info = 0;
}

// R: appl/dtrsl.f
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dtrsl_(
    t: *mut f64,
    ldt: *const c_int,
    n: *const c_int,
    b: *mut f64,
    job: *const c_int,
    info: *mut c_int,
) {
    let ldt = us(*ldt);
    let n = *n;
    let tt = |i: c_int, j: c_int| t.add((i - 1) as usize + (j - 1) as usize * ldt);
    let bb = |i: c_int| b.add((i - 1) as usize);
    let one: c_int = 1;
    for i in 1..=n {
        *info = i;
        if *tt(i, i) == 0.0 {
            return;
        }
    }
    *info = 0;
    let mut kase = 1;
    if *job % 10 != 0 {
        kase = 2;
    }
    if (*job % 100) / 10 != 0 {
        kase += 2;
    }
    match kase {
        1 => {
            *bb(1) /= *tt(1, 1);
            for j in 2..=n {
                let temp = -*bb(j - 1);
                let m = n - j + 1;
                daxpy_(&m, &temp, tt(j, j - 1), &one, bb(j), &one);
                *bb(j) /= *tt(j, j);
            }
        }
        2 => {
            *bb(n) /= *tt(n, n);
            for jj in 2..=n {
                let j = n - jj + 1;
                let temp = -*bb(j + 1);
                daxpy_(&j, &temp, tt(1, j + 1), &one, bb(1), &one);
                *bb(j) /= *tt(j, j);
            }
        }
        3 => {
            *bb(n) /= *tt(n, n);
            for jj in 2..=n {
                let j = n - jj + 1;
                let m = jj - 1;
                *bb(j) -= ddot_(&m, tt(j + 1, j), &one, bb(j + 1), &one);
                *bb(j) /= *tt(j, j);
            }
        }
        _ => {
            *bb(1) /= *tt(1, 1);
            for j in 2..=n {
                let m = j - 1;
                *bb(j) -= ddot_(&m, tt(1, j), &one, bb(1), &one);
                *bb(j) /= *tt(j, j);
            }
        }
    }
}

// R: appl/dpbfa.f (Cholesky of a banded positive definite matrix)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dpbfa_(abd: *mut f64, lda: *const c_int, n: *const c_int, m: *const c_int, info: *mut c_int) {
    let lda = us(*lda);
    let m = *m;
    let a = |i: c_int, j: c_int| abd.add((i - 1) as usize + (j - 1) as usize * lda);
    let one: c_int = 1;
    for j in 1..=*n {
        *info = j;
        let mut s = 0.0;
        let mut ik = m + 1;
        let mut jk = (j - m).max(1);
        let mu = (m + 2 - j).max(1);
        for k in mu..=m {
            let len = k - mu;
            let mut t = *a(k, j) - ddot_(&len, a(ik, jk), &one, a(mu, j), &one);
            t /= *a(m + 1, jk);
            *a(k, j) = t;
            s += t * t;
            ik -= 1;
            jk += 1;
        }
        s = *a(m + 1, j) - s;
        if s <= 0.0 {
            return;
        }
        *a(m + 1, j) = s.sqrt();
    }
    *info = 0;
}

// R: appl/dpbsl.f (solve with the factor from dpbfa)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dpbsl_(abd: *mut f64, lda: *const c_int, n: *const c_int, m: *const c_int, b: *mut f64) {
    let lda = us(*lda);
    let (n, m) = (*n, *m);
    let a = |i: c_int, j: c_int| abd.add((i - 1) as usize + (j - 1) as usize * lda);
    let bb = |i: c_int| b.add((i - 1) as usize);
    let one: c_int = 1;
    for k in 1..=n {
        let lm = (k - 1).min(m);
        let la = m + 1 - lm;
        let lb = k - lm;
        let t = ddot_(&lm, a(la, k), &one, bb(lb), &one);
        *bb(k) = (*bb(k) - t) / *a(m + 1, k);
    }
    for kb in 1..=n {
        let k = n + 1 - kb;
        let lm = (k - 1).min(m);
        let la = m + 1 - lm;
        let lb = k - lm;
        *bb(k) /= *a(m + 1, k);
        let t = -*bb(k);
        daxpy_(&lm, &t, a(la, k), &one, bb(lb), &one);
    }
}

// R: BLAS drotg (LAPACK 3.10's drotg.f90, as Debian's reference BLAS 3.12)
unsafe fn drotg(a: &mut f64, b: &mut f64, c: &mut f64, s: &mut f64) {
    let safmin = f64::MIN_POSITIVE;
    let safmax = 1.0 / safmin;
    let anorm = a.abs();
    let bnorm = b.abs();
    if bnorm == 0.0 {
        *c = 1.0;
        *s = 0.0;
        *b = 0.0;
    } else if anorm == 0.0 {
        *c = 0.0;
        *s = 1.0;
        *a = *b;
        *b = 1.0;
    } else {
        let scl = safmax.min(safmin.max(anorm).max(bnorm));
        let sigma = if anorm > bnorm { 1f64.copysign(*a) } else { 1f64.copysign(*b) };
        let (as_, bs) = (*a / scl, *b / scl);
        let r = sigma * (scl * (as_ * as_ + bs * bs).sqrt());
        *c = *a / r;
        *s = *b / r;
        let z = if anorm > bnorm {
            *s
        } else if *c != 0.0 {
            1.0 / *c
        } else {
            1.0
        };
        *a = r;
        *b = z;
    }
}

// R: BLAS drot (unit strides)
unsafe fn drot(n: c_int, x: *mut f64, y: *mut f64, c: f64, s: f64) {
    for i in 0..n.max(0) as usize {
        let t = c * *x.add(i) + s * *y.add(i);
        *y.add(i) = c * *y.add(i) - s * *x.add(i);
        *x.add(i) = t;
    }
}

// R: appl/dsvdc.f (LINPACK's singular value decomposition)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dsvdc_(
    x: *mut f64,
    ldx: *const c_int,
    n: *const c_int,
    p: *const c_int,
    s: *mut f64,
    e: *mut f64,
    u: *mut f64,
    ldu: *const c_int,
    v: *mut f64,
    ldv: *const c_int,
    work: *mut f64,
    job: *const c_int,
    info: *mut c_int,
) {
    let (ldx, ldu, ldv) = (us(*ldx), us(*ldu), us(*ldv));
    let (n, p, job) = (*n, *p, *job);
    let xa = |i: c_int, j: c_int| x.add((i - 1) as usize + (j - 1) as usize * ldx);
    let ua = |i: c_int, j: c_int| u.add((i - 1) as usize + (j - 1) as usize * ldu);
    let va = |i: c_int, j: c_int| v.add((i - 1) as usize + (j - 1) as usize * ldv);
    let sv = |i: c_int| s.add((i - 1) as usize);
    let ev = |i: c_int| e.add((i - 1) as usize);
    let wv = |i: c_int| work.add((i - 1) as usize);
    let one: c_int = 1;
    let sign = |a: f64, b: f64| if b >= 0.0 { a.abs() } else { -a.abs() };
    let maxit = 30;
    let jobu = (job % 100) / 10;
    let ncu = if jobu > 1 { n.min(p) } else { n };
    let wantu = jobu != 0;
    let wantv = job % 10 != 0;
    *info = 0;
    let nct = (n - 1).min(p);
    let nrt = 0.max((p - 2).min(n));
    let lu = nct.max(nrt);
    // reduce x to bidiagonal form
    for l in 1..=lu {
        let lp1 = l + 1;
        if l <= nct {
            let m1 = n - l + 1;
            *sv(l) = dnrm2_(&m1, xa(l, l), &one);
            if *sv(l) != 0.0 {
                if *xa(l, l) != 0.0 {
                    *sv(l) = sign(*sv(l), *xa(l, l));
                }
                let r = 1.0 / *sv(l);
                dscal_(&m1, &r, xa(l, l), &one);
                *xa(l, l) += 1.0;
            }
            *sv(l) = -*sv(l);
        }
        for j in lp1..=p {
            if l <= nct && *sv(l) != 0.0 {
                let m1 = n - l + 1;
                let t = -ddot_(&m1, xa(l, l), &one, xa(l, j), &one) / *xa(l, l);
                daxpy_(&m1, &t, xa(l, l), &one, xa(l, j), &one);
            }
            *ev(j) = *xa(l, j);
        }
        if wantu && l <= nct {
            for i in l..=n {
                *ua(i, l) = *xa(i, l);
            }
        }
        if l <= nrt {
            let m1 = p - l;
            *ev(l) = dnrm2_(&m1, ev(lp1), &one);
            if *ev(l) != 0.0 {
                if *ev(lp1) != 0.0 {
                    *ev(l) = sign(*ev(l), *ev(lp1));
                }
                let r = 1.0 / *ev(l);
                dscal_(&m1, &r, ev(lp1), &one);
                *ev(lp1) += 1.0;
            }
            *ev(l) = -*ev(l);
            if !(lp1 > n || *ev(l) == 0.0) {
                for i in lp1..=n {
                    *wv(i) = 0.0;
                }
                let m2 = n - l;
                for j in lp1..=p {
                    daxpy_(&m2, &*ev(j), xa(lp1, j), &one, wv(lp1), &one);
                }
                for j in lp1..=p {
                    let t = -*ev(j) / *ev(lp1);
                    daxpy_(&m2, &t, wv(lp1), &one, xa(lp1, j), &one);
                }
            }
            if wantv {
                for i in lp1..=p {
                    *va(i, l) = *ev(i);
                }
            }
        }
    }
    // set up the final bidiagonal matrix of order m
    let mut m = p.min(n + 1);
    let nctp1 = nct + 1;
    let nrtp1 = nrt + 1;
    if nct < p {
        *sv(nctp1) = *xa(nctp1, nctp1);
    }
    if n < m {
        *sv(m) = 0.0;
    }
    if nrtp1 < m {
        *ev(nrtp1) = *xa(nrtp1, m);
    }
    *ev(m) = 0.0;
    // generate u
    if wantu {
        for j in nctp1..=ncu {
            for i in 1..=n {
                *ua(i, j) = 0.0;
            }
            *ua(j, j) = 1.0;
        }
        for ll in 1..=nct {
            let l = nct - ll + 1;
            if *sv(l) != 0.0 {
                let lp1 = l + 1;
                let m1 = n - l + 1;
                for j in lp1..=ncu {
                    let t = -ddot_(&m1, ua(l, l), &one, ua(l, j), &one) / *ua(l, l);
                    daxpy_(&m1, &t, ua(l, l), &one, ua(l, j), &one);
                }
                let mone = -1.0;
                dscal_(&m1, &mone, ua(l, l), &one);
                *ua(l, l) += 1.0;
                for i in 1..l {
                    *ua(i, l) = 0.0;
                }
            } else {
                for i in 1..=n {
                    *ua(i, l) = 0.0;
                }
                *ua(l, l) = 1.0;
            }
        }
    }
    // generate v
    if wantv {
        for ll in 1..=p {
            let l = p - ll + 1;
            let lp1 = l + 1;
            if l <= nrt && *ev(l) != 0.0 {
                let m1 = p - l;
                for j in lp1..=p {
                    let t = -ddot_(&m1, va(lp1, l), &one, va(lp1, j), &one) / *va(lp1, l);
                    daxpy_(&m1, &t, va(lp1, l), &one, va(lp1, j), &one);
                }
            }
            for i in 1..=p {
                *va(i, l) = 0.0;
            }
            *va(l, l) = 1.0;
        }
    }
    // main iteration loop for the singular values
    let mm = m;
    let mut iter = 0;
    loop {
        if m == 0 {
            return;
        }
        if iter >= maxit {
            *info = m;
            return;
        }
        // l: the start of the unreduced part
        let mut l = 0;
        for ll in 1..=m {
            l = m - ll;
            if l == 0 {
                break;
            }
            let test = (*sv(l)).abs() + (*sv(l + 1)).abs();
            let ztest = test + (*ev(l)).abs();
            let acc = (test - ztest).abs() / (1.0e-100 + test);
            // Fortran: `if (acc .gt. 1.d-15) goto 380` (NaN falls through)
            if !(acc > 1.0e-15) {
                *ev(l) = 0.0;
                break;
            }
        }
        let kase;
        if l == m - 1 {
            kase = 4;
        } else {
            let lp1 = l + 1;
            let mp1 = m + 1;
            let mut ls = l;
            for lls in lp1..=mp1 {
                ls = m - lls + lp1;
                if ls == l {
                    break;
                }
                let mut test = 0.0;
                if ls != m {
                    test += (*ev(ls)).abs();
                }
                if ls != l + 1 {
                    test += (*ev(ls - 1)).abs();
                }
                let ztest = test + (*sv(ls)).abs();
                let acc = (test - ztest).abs() / (1.0e-100 + test);
                if !(acc > 1.0e-15) {
                    *sv(ls) = 0.0;
                    break;
                }
            }
            if ls == l {
                kase = 3;
            } else if ls == m {
                kase = 1;
            } else {
                kase = 2;
                l = ls;
            }
        }
        l += 1;
        let (mut cs, mut sn) = (0.0, 0.0);
        match kase {
            1 => {
                // deflate negligible s(m)
                let mm1 = m - 1;
                let mut f = *ev(m - 1);
                *ev(m - 1) = 0.0;
                for kk in l..=mm1 {
                    let k = mm1 - kk + l;
                    let mut t1 = *sv(k);
                    drotg(&mut t1, &mut f, &mut cs, &mut sn);
                    *sv(k) = t1;
                    if k != l {
                        f = -sn * *ev(k - 1);
                        *ev(k - 1) *= cs;
                    }
                    if wantv {
                        drot(p, va(1, k), va(1, m), cs, sn);
                    }
                }
            }
            2 => {
                // split at negligible s(l)
                let mut f = *ev(l - 1);
                *ev(l - 1) = 0.0;
                for k in l..=m {
                    let mut t1 = *sv(k);
                    drotg(&mut t1, &mut f, &mut cs, &mut sn);
                    *sv(k) = t1;
                    f = -sn * *ev(k);
                    *ev(k) *= cs;
                    if wantu {
                        drot(n, ua(1, k), ua(1, l - 1), cs, sn);
                    }
                }
            }
            3 => {
                // one QR step: the shift
                let scale = (*sv(m))
                    .abs()
                    .max((*sv(m - 1)).abs())
                    .max((*ev(m - 1)).abs())
                    .max((*sv(l)).abs())
                    .max((*ev(l)).abs());
                let sm = *sv(m) / scale;
                let smm1 = *sv(m - 1) / scale;
                let emm1 = *ev(m - 1) / scale;
                let sl = *sv(l) / scale;
                let el = *ev(l) / scale;
                let b = ((smm1 + sm) * (smm1 - sm) + emm1 * emm1) / 2.0;
                let c = (sm * emm1) * (sm * emm1);
                let mut shift = 0.0;
                if !(b == 0.0 && c == 0.0) {
                    shift = (b * b + c).sqrt();
                    if b < 0.0 {
                        shift = -shift;
                    }
                    shift = c / (b + shift);
                }
                let mut f = (sl + sm) * (sl - sm) + shift;
                let mut g = sl * el;
                let mm1 = m - 1;
                for k in l..=mm1 {
                    drotg(&mut f, &mut g, &mut cs, &mut sn);
                    if k != l {
                        *ev(k - 1) = f;
                    }
                    f = cs * *sv(k) + sn * *ev(k);
                    *ev(k) = cs * *ev(k) - sn * *sv(k);
                    g = sn * *sv(k + 1);
                    *sv(k + 1) *= cs;
                    if wantv {
                        drot(p, va(1, k), va(1, k + 1), cs, sn);
                    }
                    drotg(&mut f, &mut g, &mut cs, &mut sn);
                    *sv(k) = f;
                    f = cs * *ev(k) + sn * *sv(k + 1);
                    *sv(k + 1) = -sn * *ev(k) + cs * *sv(k + 1);
                    g = sn * *ev(k + 1);
                    *ev(k + 1) *= cs;
                    if wantu && k < n {
                        drot(n, ua(1, k), ua(1, k + 1), cs, sn);
                    }
                }
                *ev(m - 1) = f;
                iter += 1;
            }
            _ => {
                // convergence: make the singular value non-negative, order
                if !(*sv(l) >= 0.0) {
                    *sv(l) = -*sv(l);
                    if wantv {
                        let mone = -1.0;
                        dscal_(&p, &mone, va(1, l), &one);
                    }
                }
                while l != mm && !(*sv(l) >= *sv(l + 1)) {
                    std::ptr::swap(sv(l), sv(l + 1));
                    if wantv && l < p {
                        dswap(p.max(0) as usize, va(1, l), va(1, l + 1));
                    }
                    if wantu && l < n {
                        dswap(n.max(0) as usize, ua(1, l), ua(1, l + 1));
                    }
                    l += 1;
                }
                iter = 0;
                m -= 1;
            }
        }
    }
}
