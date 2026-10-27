//! Reference BLAS level 1 (Netlib 3.12, as linked by GNU R on Debian) and
//! the LINPACK QR routines R ships in src/appl (dqrdc2.f, dqrsl.f,
//! dqrls.f, dqrutl.f). The BLAS loops keep their unrolling: it fixes the
//! order of floating-point operations, so results match R's bit for bit.
//!
//! Matrices are column-major slices with a leading dimension; vector
//! arguments are slices starting at the Fortran element they name.

/// `ddot(n, dx, 1, dy, 1)`.
// BLAS: ddot.f (unit strides, unrolled by 5)
pub fn ddot(n: usize, dx: &[f64], dy: &[f64]) -> f64 {
    let mut dtemp = 0.0;
    if n == 0 {
        return dtemp;
    }
    let m = n % 5;
    for i in 0..m {
        dtemp += dx[i] * dy[i];
    }
    if n < 5 {
        return dtemp;
    }
    let mut i = m;
    while i < n {
        dtemp = dtemp
            + dx[i] * dy[i]
            + dx[i + 1] * dy[i + 1]
            + dx[i + 2] * dy[i + 2]
            + dx[i + 3] * dy[i + 3]
            + dx[i + 4] * dy[i + 4];
        i += 5;
    }
    dtemp
}

/// `daxpy(n, da, dx, 1, dy, 1)`: dy += da * dx.
// BLAS: daxpy.f (unit strides, unrolled by 4)
pub fn daxpy(n: usize, da: f64, dx: &[f64], dy: &mut [f64]) {
    if n == 0 || da == 0.0 {
        return;
    }
    let m = n % 4;
    for i in 0..m {
        dy[i] += da * dx[i];
    }
    if n < 4 {
        return;
    }
    let mut i = m;
    while i < n {
        dy[i] += da * dx[i];
        dy[i + 1] += da * dx[i + 1];
        dy[i + 2] += da * dx[i + 2];
        dy[i + 3] += da * dx[i + 3];
        i += 4;
    }
}

/// `dscal(n, da, dx, 1)`.
// BLAS: dscal.f
pub fn dscal(n: usize, da: f64, dx: &mut [f64]) {
    for v in dx.iter_mut().take(n) {
        *v *= da;
    }
}

/// `dnrm2(n, x, 1)` with Blue's scaling.
// BLAS: dnrm2.f90 (LAPACK 3.10+)
pub fn dnrm2(n: usize, x: &[f64]) -> f64 {
    let tsml = 2f64.powi(-511);
    let tbig = 2f64.powi(486);
    let ssml = 2f64.powi(537);
    let sbig = 2f64.powi(-538);
    let maxn = f64::MAX;
    if n == 0 {
        return 0.0;
    }
    let mut notbig = true;
    let (mut asml, mut amed, mut abig) = (0.0f64, 0.0f64, 0.0f64);
    for &v in x.iter().take(n) {
        let ax = v.abs();
        if ax > tbig {
            abig += (ax * sbig) * (ax * sbig);
            notbig = false;
        } else if ax < tsml {
            if notbig {
                asml += (ax * ssml) * (ax * ssml);
            }
        } else {
            amed += ax * ax;
        }
    }
    let (scl, sumsq);
    if abig > 0.0 {
        if amed > 0.0 || amed > maxn || amed.is_nan() {
            abig += (amed * sbig) * sbig;
        }
        scl = 1.0 / sbig;
        sumsq = abig;
    } else if asml > 0.0 {
        if amed > 0.0 || amed > maxn || amed.is_nan() {
            let amed2 = amed.sqrt();
            let asml2 = asml.sqrt() / ssml;
            let (ymin, ymax) = if asml2 > amed2 { (amed2, asml2) } else { (asml2, amed2) };
            scl = 1.0;
            sumsq = ymax * ymax * (1.0 + (ymin / ymax) * (ymin / ymax));
        } else {
            scl = 1.0 / ssml;
            sumsq = asml;
        }
    } else {
        scl = 1.0;
        sumsq = amed;
    }
    scl * sumsq.sqrt()
}

/// Fortran's `sign(a, b)`.
fn sign(a: f64, b: f64) -> f64 {
    if b.is_sign_negative() { -a.abs() } else { a.abs() }
}

/// QR decomposition with R's limited column pivoting. Returns the rank.
/// `x` is `ldx` by `p`; `work` has `2 * p` elements.
// R: appl/dqrdc2.f
#[allow(clippy::too_many_arguments)]
pub fn dqrdc2(
    x: &mut [f64],
    ldx: usize,
    n: usize,
    p: usize,
    tol: f64,
    qraux: &mut [f64],
    jpvt: &mut [i32],
    work: &mut [f64],
) -> usize {
    let at = |i: usize, j: usize| i + j * ldx; // 0-based (i, j)
    // work(j,1) = work[j], work(j,2) = work[p + j]
    if n > 0 {
        for j in 0..p {
            qraux[j] = dnrm2(n, &x[at(0, j)..]);
            work[j] = qraux[j];
            work[p + j] = qraux[j];
            if work[p + j] == 0.0 {
                work[p + j] = 1.0;
            }
        }
    }
    let lup = n.min(p);
    let mut k = p + 1;
    for l in 1..=lup {
        // Move negligible columns to the end.
        loop {
            if l >= k || qraux[l - 1] >= work[p + l - 1] * tol {
                break;
            }
            for i in 0..n {
                let t = x[at(i, l - 1)];
                for j in (l + 1)..=p {
                    x[at(i, j - 2)] = x[at(i, j - 1)];
                }
                x[at(i, p - 1)] = t;
            }
            let i = jpvt[l - 1];
            let t = qraux[l - 1];
            let tt = work[l - 1];
            let ttt = work[p + l - 1];
            for j in (l + 1)..=p {
                jpvt[j - 2] = jpvt[j - 1];
                qraux[j - 2] = qraux[j - 1];
                work[j - 2] = work[j - 1];
                work[p + j - 2] = work[p + j - 1];
            }
            jpvt[p - 1] = i;
            qraux[p - 1] = t;
            work[p - 1] = tt;
            work[2 * p - 1] = ttt;
            k -= 1;
        }
        if l == n {
            continue;
        }
        let ll = l - 1;
        let col = at(ll, ll);
        let mut nrmxl = dnrm2(n - l + 1, &x[col..]);
        if nrmxl == 0.0 {
            continue;
        }
        if x[col] != 0.0 {
            nrmxl = sign(nrmxl, x[col]);
        }
        dscal(n - l + 1, 1.0 / nrmxl, &mut x[col..]);
        x[col] += 1.0;
        for j in (l + 1)..=p {
            let cj = at(ll, j - 1);
            let (head, tail) = x.split_at_mut(cj);
            let t = -ddot(n - l + 1, &head[col..], tail) / head[col];
            daxpy(n - l + 1, t, &head[col..], tail);
            if qraux[j - 1] != 0.0 {
                let r = tail[0].abs() / qraux[j - 1];
                let tt = (1.0 - r * r).max(0.0);
                let t = tt;
                if t.abs() >= 1e-6 {
                    qraux[j - 1] *= t.sqrt();
                } else {
                    qraux[j - 1] = dnrm2(n - l, &tail[1..]);
                    work[j - 1] = qraux[j - 1];
                }
            }
        }
        qraux[ll] = x[col];
        x[col] = -nrmxl;
    }
    (k - 1).min(n)
}

/// Outputs of [`dqrsl`] that the job selects; each is the full vector.
#[derive(Default)]
pub struct Qrsl<'a> {
    pub qy: Option<&'a mut [f64]>,
    pub qty: Option<&'a mut [f64]>,
    pub b: Option<&'a mut [f64]>,
    pub rsd: Option<&'a mut [f64]>,
    pub xb: Option<&'a mut [f64]>,
}

/// Apply the output of [`dqrdc2`]: Q y, Q'y, coefficients, residuals and
/// fitted values according to `job`. Returns `info`.
// R: appl/dqrsl.f
#[allow(clippy::too_many_arguments)]
pub fn dqrsl(x: &mut [f64], ldx: usize, n: usize, k: usize, qraux: &[f64], y: &[f64], o: &mut Qrsl, job: i32) -> i32 {
    let at = |i: usize, j: usize| i + j * ldx;
    let mut info = 0;
    let cqy = job / 10000 != 0;
    let cqty = job % 10000 != 0;
    let cb = (job % 1000) / 100 != 0;
    let cr = (job % 100) / 10 != 0;
    let cxb = job % 10 != 0;
    let ju = k.min(n.saturating_sub(1));
    macro_rules! out {
        ($f:ident) => {
            o.$f.as_deref_mut().expect(concat!("dqrsl: ", stringify!($f)))
        };
    }
    if ju == 0 {
        if cqy {
            out!(qy)[0] = y[0];
        }
        if cqty {
            out!(qty)[0] = y[0];
        }
        if cxb {
            out!(xb)[0] = y[0];
        }
        if cb {
            if x[0] != 0.0 {
                out!(b)[0] = y[0] / x[0];
            } else {
                info = 1;
            }
        }
        if cr {
            out!(rsd)[0] = 0.0;
        }
        return info;
    }
    if cqy {
        out!(qy)[..n].copy_from_slice(&y[..n]);
    }
    if cqty {
        out!(qty)[..n].copy_from_slice(&y[..n]);
    }
    // Apply the Householder transformation of column j (1-based) to v.
    let house = |x: &mut [f64], j: usize, v: &mut [f64]| {
        if qraux[j - 1] == 0.0 {
            return;
        }
        let d = at(j - 1, j - 1);
        let temp = x[d];
        x[d] = qraux[j - 1];
        let t = -ddot(n - j + 1, &x[d..], &v[j - 1..]) / x[d];
        daxpy(n - j + 1, t, &x[d..], &mut v[j - 1..]);
        x[d] = temp;
    };
    if cqy {
        let qy = out!(qy);
        for jj in 1..=ju {
            house(x, ju - jj + 1, qy);
        }
    }
    if cqty {
        let qty = out!(qty);
        for j in 1..=ju {
            house(x, j, qty);
        }
    }
    let qty_copy: Vec<f64> = if cqty { out!(qty)[..n].to_vec() } else { vec![] };
    if cb {
        out!(b)[..k].copy_from_slice(&qty_copy[..k]);
    }
    if cxb {
        out!(xb)[..k].copy_from_slice(&qty_copy[..k]);
    }
    if cr && k < n {
        out!(rsd)[k..n].copy_from_slice(&qty_copy[k..n]);
    }
    if cxb && k < n {
        for v in out!(xb)[k..n].iter_mut() {
            *v = 0.0;
        }
    }
    if cr {
        for v in out!(rsd)[..k].iter_mut() {
            *v = 0.0;
        }
    }
    if cb {
        let b = out!(b);
        for jj in 1..=k {
            let j = k - jj + 1;
            let d = at(j - 1, j - 1);
            if x[d] == 0.0 {
                info = j as i32;
                break;
            }
            b[j - 1] /= x[d];
            if j != 1 {
                let t = -b[j - 1];
                daxpy(j - 1, t, &x[at(0, j - 1)..], b);
            }
        }
    }
    if cr || cxb {
        for jj in 1..=ju {
            let j = ju - jj + 1;
            if qraux[j - 1] == 0.0 {
                continue;
            }
            if cr {
                house(x, j, out!(rsd));
            }
            if cxb {
                house(x, j, out!(xb));
            }
        }
    }
    info
}

/// Least squares fit by QR. Returns the rank.
// R: appl/dqrls.f
#[allow(clippy::too_many_arguments)]
pub fn dqrls(
    x: &mut [f64],
    n: usize,
    p: usize,
    y: &[f64],
    ny: usize,
    tol: f64,
    b: &mut [f64],
    rsd: &mut [f64],
    qty: &mut [f64],
    jpvt: &mut [i32],
    qraux: &mut [f64],
    work: &mut [f64],
) -> usize {
    let k = dqrdc2(x, n, n, p, tol, qraux, jpvt, work);
    if k > 0 {
        for jj in 0..ny {
            let yc = &y[jj * n..(jj + 1) * n];
            let mut o = Qrsl {
                qty: Some(&mut qty[jj * n..(jj + 1) * n]),
                b: Some(&mut b[jj * p..(jj + 1) * p]),
                rsd: Some(&mut rsd[jj * n..(jj + 1) * n]),
                ..Default::default()
            };
            dqrsl(x, n, n, k, qraux, yc, &mut o, 1110);
        }
    } else {
        rsd[..n * ny].copy_from_slice(&y[..n * ny]);
    }
    for jj in 0..ny {
        for j in k..p {
            b[j + jj * p] = 0.0;
        }
    }
    k
}

/// The dqrutl.f drivers: apply [`dqrsl`] with `job` to each column of
/// `y` (n by ny), returning the selected output for every column. For
/// `qty`-based jobs (`dqrcf`, `dqrrsd`, `dqrxb`) `y` is overwritten with
/// Q'y, as in R where `qty` aliases `y`.
// R: appl/dqrutl.f
pub enum Util {
    Qty,
    Qy,
    Coef,
    Rsd,
    Xb,
}

#[allow(clippy::too_many_arguments)]
pub fn dqrutl(
    what: Util,
    x: &mut [f64],
    n: usize,
    k: usize,
    qraux: &[f64],
    y: &mut [f64],
    ny: usize,
    out: &mut [f64],
) -> i32 {
    let mut info = 0;
    for j in 0..ny {
        let ycol: Vec<f64> = y[j * n..(j + 1) * n].to_vec();
        match what {
            Util::Qty => {
                let mut o = Qrsl { qty: Some(&mut out[j * n..(j + 1) * n]), ..Default::default() };
                info = dqrsl(x, n, n, k, qraux, &ycol, &mut o, 1000);
            }
            Util::Qy => {
                let mut o = Qrsl { qy: Some(&mut out[j * n..(j + 1) * n]), ..Default::default() };
                info = dqrsl(x, n, n, k, qraux, &ycol, &mut o, 10000);
            }
            Util::Coef => {
                let mut o = Qrsl {
                    qty: Some(&mut y[j * n..(j + 1) * n]),
                    b: Some(&mut out[j * k..(j + 1) * k]),
                    ..Default::default()
                };
                info = dqrsl(x, n, n, k, qraux, &ycol, &mut o, 100);
            }
            Util::Rsd => {
                let mut o = Qrsl {
                    qty: Some(&mut y[j * n..(j + 1) * n]),
                    rsd: Some(&mut out[j * n..(j + 1) * n]),
                    ..Default::default()
                };
                info = dqrsl(x, n, n, k, qraux, &ycol, &mut o, 10);
            }
            Util::Xb => {
                let mut o = Qrsl {
                    qty: Some(&mut y[j * n..(j + 1) * n]),
                    xb: Some(&mut out[j * n..(j + 1) * n]),
                    ..Default::default()
                };
                info = dqrsl(x, n, n, k, qraux, &ycol, &mut o, 1);
            }
        }
    }
    info
}
