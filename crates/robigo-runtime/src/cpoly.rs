//! GNU R's complex polynomial zero finder, used by `polyroot()`.
//!
//! R: main/complex.c:R_cpolyroot (formerly src/appl/cpoly.c), a C
//! translation of TOMS Algorithm 419 (Jenkins and Traub, Comm. ACM 15
//! (1972) 97-99) by Ross Ihaka and the R Core Team, together with its
//! helpers `noshft`, `fxshft`, `vrshft`, `calct`, `nexth`, `polyev`,
//! `errev`, `cpoly_cauchy`, `cpoly_scale` and `cdivid`. The C file keeps
//! its varying coefficient vectors in file-scope statics; here they live
//! in [`Cpoly`] and every step is otherwise the same arithmetic in the
//! same order, so the zeros are bit-identical to GNU R's.

const ETA: f64 = f64::EPSILON;
const ARE: f64 = f64::EPSILON;
const MRE: f64 = 2.0 * std::f64::consts::SQRT_2 * f64::EPSILON;
const INFIN: f64 = f64::MAX;
const SMALNO: f64 = f64::MIN_POSITIVE;
/// FLT_RADIX.
const BASE: f64 = 2.0;
/// cos 94, sin 94: the shift is rotated 94 degrees from the previous one.
const COSR: f64 = -0.06975647374412529990;
const SINR: f64 = 0.99756405025982424767;

// C's `vrshft` keeps `relstp` in a function-static that persists across
// calls; keeping that value too avoids diverging when a call starts with
// h(s) == 0 at the first iterate.
thread_local! {
    static RELSTP: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// Find the zeros of the polynomial whose coefficients, in order of
/// decreasing powers, are `opr[i] + opi[i]*I` for `i in 0..=degree`.
///
/// `zeror`/`zeroi` receive `degree` zeros. Returns `true` when the root
/// finder failed (R's `fail`), which `do_polyroot` turns into the error
/// "root finding code failed".
// R: complex.c:R_cpolyroot
pub fn cpolyroot(opr: &[f64], opi: &[f64], degree: usize, zeror: &mut [f64], zeroi: &mut [f64]) -> bool {
    let mut nn = degree as isize;
    let d1 = nn - 1;
    let mut xx = std::f64::consts::FRAC_1_SQRT_2;
    let mut yy = -xx;

    // The algorithm fails if the leading coefficient is zero.
    if opr[0] == 0.0 && opi[0] == 0.0 {
        return true;
    }

    // Remove the zeros at the origin, if any.
    while opr[nn as usize] == 0.0 && opi[nn as usize] == 0.0 {
        let d_n = d1 - nn + 1;
        zeror[d_n as usize] = 0.0;
        zeroi[d_n as usize] = 0.0;
        nn -= 1;
    }
    nn += 1;
    // Now nn = #{coefficients} = (relevant degree) + 1.
    if nn == 1 {
        return false;
    }

    let n = nn as usize;
    let mut c = Cpoly {
        nn,
        pr: vec![0.0; n],
        pi: vec![0.0; n],
        hr: vec![0.0; n],
        hi: vec![0.0; n],
        qpr: vec![0.0; n],
        qpi: vec![0.0; n],
        qhr: vec![0.0; n],
        qhi: vec![0.0; n],
        shr: vec![0.0; n],
        shi: vec![0.0; n],
        sr: 0.0,
        si: 0.0,
        tr: 0.0,
        ti: 0.0,
        pvr: 0.0,
        pvi: 0.0,
    };
    for i in 0..n {
        c.pr[i] = opr[i];
        c.pi[i] = opi[i];
        c.shr[i] = c.pr[i].hypot(c.pi[i]);
    }

    // Scale the polynomial with factor `bnd`.
    let bnd = cpoly_scale(n, &c.shr, ETA, INFIN, SMALNO, BASE);
    if bnd != 1.0 {
        for i in 0..n {
            c.pr[i] *= bnd;
            c.pi[i] *= bnd;
        }
    }

    // Start the algorithm for one zero.
    'deflate: while c.nn > 2 {
        // A lower bound on the modulus of the zeros.
        for i in 0..c.nn as usize {
            c.shr[i] = c.pr[i].hypot(c.pi[i]);
        }
        let bnd = cpoly_cauchy(c.nn as usize, &mut c.shr, &mut c.shi);

        let (zr, zi);
        // Outer loop for two major passes with different shift sequences.
        for _i1 in 1..=2 {
            // First stage: no shift.
            c.noshft(5);

            // Inner loop to select a shift.
            for i2 in 1..=9 {
                let xxx = COSR * xx - SINR * yy;
                yy = SINR * xx + COSR * yy;
                xx = xxx;
                c.sr = bnd * xx;
                c.si = bnd * yy;

                // Second stage: fixed shift.
                if let Some(z) = c.fxshft(i2 * 10) {
                    (zr, zi) = z;
                    // The second stage jumps straight to the third-stage
                    // iteration; on success the zero is stored and the
                    // polynomial deflated.
                    let d_n = d1 + 2 - c.nn;
                    zeror[d_n as usize] = zr;
                    zeroi[d_n as usize] = zi;
                    c.nn -= 1;
                    for i in 0..c.nn as usize {
                        c.pr[i] = c.qpr[i];
                        c.pi[i] = c.qpi[i];
                    }
                    continue 'deflate;
                }
            }
        }

        // The zero finder failed on two major passes.
        return true;
    }

    // Calculate the final zero and return.
    let (zr, zi) = cdivid(-c.pr[1], -c.pi[1], c.pr[0], c.pi[0]);
    zeror[d1 as usize] = zr;
    zeroi[d1 as usize] = zi;
    false
}

struct Cpoly {
    nn: isize,
    pr: Vec<f64>,
    pi: Vec<f64>,
    hr: Vec<f64>,
    hi: Vec<f64>,
    qpr: Vec<f64>,
    qpi: Vec<f64>,
    qhr: Vec<f64>,
    qhi: Vec<f64>,
    shr: Vec<f64>,
    shi: Vec<f64>,
    sr: f64,
    si: f64,
    tr: f64,
    ti: f64,
    pvr: f64,
    pvi: f64,
}

impl Cpoly {
    /// Computes the derivative polynomial as the initial polynomial and
    /// `l1` no-shift h polynomials.
    // R: complex.c:noshft
    fn noshft(&mut self, l1: i32) {
        let n = self.nn - 1;
        let nm1 = n - 1;
        for i in 0..n as usize {
            let xni = (self.nn - i as isize - 1) as f64;
            self.hr[i] = xni * self.pr[i] / n as f64;
            self.hi[i] = xni * self.pi[i] / n as f64;
        }

        for _ in 1..=l1 {
            if self.hr[nm1 as usize].hypot(self.hi[nm1 as usize])
                <= ETA * 10.0 * self.pr[nm1 as usize].hypot(self.pi[nm1 as usize])
            {
                // If the constant term is essentially zero, shift h
                // coefficients.
                for i in 1..=nm1 {
                    let j = self.nn - i;
                    self.hr[j as usize - 1] = self.hr[j as usize - 2];
                    self.hi[j as usize - 1] = self.hi[j as usize - 2];
                }
                self.hr[0] = 0.0;
                self.hi[0] = 0.0;
            } else {
                let (tr, ti) =
                    cdivid(-self.pr[n as usize], -self.pi[n as usize], self.hr[nm1 as usize], self.hi[nm1 as usize]);
                self.tr = tr;
                self.ti = ti;
                for i in 1..=nm1 {
                    let j = (self.nn - i) as usize;
                    let t1 = self.hr[j - 2];
                    let t2 = self.hi[j - 2];
                    self.hr[j - 1] = tr * t1 - ti * t2 + self.pr[j - 1];
                    self.hi[j - 1] = tr * t2 + ti * t1 + self.pi[j - 1];
                }
                self.hr[0] = self.pr[0];
                self.hi[0] = self.pi[0];
            }
        }
    }

    /// Computes `l2` fixed-shift h polynomials, tests for convergence,
    /// and starts the variable-shift iteration; returns the approximate
    /// zero on success.
    // R: complex.c:fxshft
    fn fxshft(&mut self, l2: i32) -> Option<(f64, f64)> {
        let n = self.nn - 1;
        let mut zr = 0.0;
        let mut zi = 0.0;

        // Evaluate p at s.
        let (pvr, pvi) = polyev(self.nn as usize, self.sr, self.si, &self.pr, &self.pi, &mut self.qpr, &mut self.qpi);
        self.pvr = pvr;
        self.pvi = pvi;

        let mut test = true;
        let mut pasd = false;

        // Calculate first t = -p(s)/h(s).
        let mut h_s_0 = self.calct();

        // Main loop for one second-stage step.
        for j in 1..=l2 {
            let otr = self.tr;
            let oti = self.ti;

            // Compute the next h polynomial and the new t.
            self.nexth(h_s_0);
            h_s_0 = self.calct();
            zr = self.sr + self.tr;
            zi = self.si + self.ti;

            // Test for convergence unless stage 3 has failed once or this
            // is the last h polynomial.
            if !h_s_0 && test && j != l2 {
                if (self.tr - otr).hypot(self.ti - oti) >= zr.hypot(zi) * 0.5 {
                    pasd = false;
                } else if !pasd {
                    pasd = true;
                } else {
                    // The weak convergence test has been passed twice:
                    // start the third-stage iteration after saving the
                    // current h polynomial and shift.
                    for i in 0..n as usize {
                        self.shr[i] = self.hr[i];
                        self.shi[i] = self.hi[i];
                    }
                    let svsr = self.sr;
                    let svsi = self.si;
                    if let Some(z) = self.vrshft(10, zr, zi) {
                        return Some(z);
                    }

                    // The iteration failed to converge: turn off testing
                    // and restore h, s, pv and t.
                    test = false;
                    for i in 1..=n as usize {
                        self.hr[i - 1] = self.shr[i - 1];
                        self.hi[i - 1] = self.shi[i - 1];
                    }
                    self.sr = svsr;
                    self.si = svsi;
                    let (pvr, pvi) =
                        polyev(self.nn as usize, self.sr, self.si, &self.pr, &self.pi, &mut self.qpr, &mut self.qpi);
                    self.pvr = pvr;
                    self.pvi = pvi;
                    h_s_0 = self.calct();
                }
            }
        }

        // Attempt an iteration with the final h polynomial from stage 2.
        self.vrshft(10, zr, zi)
    }

    /// Carries out the third-stage iteration.
    // R: complex.c:vrshft
    fn vrshft(&mut self, l3: i32, zr: f64, zi: f64) -> Option<(f64, f64)> {
        let mut b = false;
        let mut omp = 0.0;
        // `relstp` is a function-static in the C file; it outlives this
        // call and even the polynomial, so it lives in a module static.
        let mut relstp = RELSTP.with(|r| r.get());
        self.sr = zr;
        self.si = zi;

        // Main loop for stage three.
        for i in 1..=l3 {
            // Evaluate p at s and test for convergence.
            let (pvr, pvi) =
                polyev(self.nn as usize, self.sr, self.si, &self.pr, &self.pi, &mut self.qpr, &mut self.qpi);
            self.pvr = pvr;
            self.pvi = pvi;

            let mp = self.pvr.hypot(self.pvi);
            let ms = self.sr.hypot(self.si);
            if mp <= 20.0 * errev(self.nn as usize, &self.qpr, &self.qpi, ms, mp, ARE, MRE) {
                return Some((self.sr, self.si));
            }

            let mut stalled = false;
            if i != 1 {
                if !b && mp >= omp && relstp < 0.05 {
                    // The iteration has stalled: probably a cluster of
                    // zeros. Do 5 fixed-shift steps into the cluster to
                    // force one zero to dominate.
                    let mut tp = relstp;
                    b = true;
                    if relstp < ETA {
                        tp = ETA;
                    }
                    let r1 = tp.sqrt();
                    let r2 = self.sr * (r1 + 1.0) - self.si * r1;
                    self.si = self.sr * r1 + self.si * (r1 + 1.0);
                    self.sr = r2;
                    let (pvr, pvi) =
                        polyev(self.nn as usize, self.sr, self.si, &self.pr, &self.pi, &mut self.qpr, &mut self.qpi);
                    self.pvr = pvr;
                    self.pvi = pvi;
                    for _j in 1..=5 {
                        let h = self.calct();
                        self.nexth(h);
                    }
                    omp = INFIN;
                    // The C code jumps straight to the next iterate.
                    stalled = true;
                } else if mp * 0.1 > omp {
                    // Exit if the polynomial value increases significantly.
                    return None;
                }
            }
            if !stalled {
                omp = mp;
            }
            // Calculate the next iterate.
            let mut h_s_0 = self.calct();
            self.nexth(h_s_0);
            h_s_0 = self.calct();
            if !h_s_0 {
                relstp = self.tr.hypot(self.ti) / self.sr.hypot(self.si);
                RELSTP.with(|r| r.set(relstp));
                self.sr += self.tr;
                self.si += self.ti;
            }
        }
        None
    }

    /// Computes `t = -p(s)/h(s)`; returns whether h(s) is essentially
    /// zero.
    // R: complex.c:calct
    fn calct(&mut self) -> bool {
        let n = (self.nn - 1) as usize;
        let (hvr, hvi) = polyev(n, self.sr, self.si, &self.hr, &self.hi, &mut self.qhr, &mut self.qhi);
        let h_s_0 = hvr.hypot(hvi) <= ARE * 10.0 * self.hr[n - 1].hypot(self.hi[n - 1]);
        if !h_s_0 {
            let (tr, ti) = cdivid(-self.pvr, -self.pvi, hvr, hvi);
            self.tr = tr;
            self.ti = ti;
        } else {
            self.tr = 0.0;
            self.ti = 0.0;
        }
        h_s_0
    }

    /// Calculates the next shifted h polynomial.
    // R: complex.c:nexth
    fn nexth(&mut self, h_s_0: bool) {
        let n = (self.nn - 1) as usize;
        if !h_s_0 {
            for j in 1..n {
                let t1 = self.qhr[j - 1];
                let t2 = self.qhi[j - 1];
                self.hr[j] = self.tr * t1 - self.ti * t2 + self.qpr[j];
                self.hi[j] = self.tr * t2 + self.ti * t1 + self.qpi[j];
            }
            self.hr[0] = self.qpr[0];
            self.hi[0] = self.qpi[0];
        } else {
            // If h(s) is zero, replace h with qh.
            for j in 1..n {
                self.hr[j] = self.qhr[j - 1];
                self.hi[j] = self.qhi[j - 1];
            }
            self.hr[0] = 0.0;
            self.hi[0] = 0.0;
        }
    }
}

/// Evaluates a polynomial `p` at `s` by the Horner recurrence, placing the
/// partial sums in `q` and the computed value in the returned pair.
// R: complex.c:polyev
#[allow(clippy::too_many_arguments)]
fn polyev(n: usize, s_r: f64, s_i: f64, p_r: &[f64], p_i: &[f64], q_r: &mut [f64], q_i: &mut [f64]) -> (f64, f64) {
    q_r[0] = p_r[0];
    q_i[0] = p_i[0];
    let mut v_r = q_r[0];
    let mut v_i = q_i[0];
    for i in 1..n {
        let t = v_r * s_r - v_i * s_i + p_r[i];
        v_i = v_r * s_i + v_i * s_r + p_i[i];
        v_r = t;
        q_i[i] = v_i;
        q_r[i] = v_r;
    }
    (v_r, v_i)
}

/// Bounds the error in evaluating the polynomial by the Horner recurrence.
// R: complex.c:errev
#[allow(clippy::too_many_arguments)]
fn errev(n: usize, qr: &[f64], qi: &[f64], ms: f64, mp: f64, a_re: f64, m_re: f64) -> f64 {
    let mut e = qr[0].hypot(qi[0]) * m_re / (a_re + m_re);
    for i in 0..n {
        e = e * ms + qr[i].hypot(qi[i]);
    }
    e * (a_re + m_re) - mp * m_re
}

/// Computes a lower bound on the moduli of the zeros of a polynomial.
/// `pot[i]` is the modulus of coefficient `i`.
// R: complex.c:cpoly_cauchy
fn cpoly_cauchy(n: usize, pot: &mut [f64], q: &mut [f64]) -> f64 {
    let n1 = n - 1;
    pot[n1] = -pot[n1];

    // Compute an upper estimate of the bound.
    let mut x = (((-pot[n1]).ln() - pot[0].ln()) / n1 as f64).exp();

    // If a Newton step at the origin is better, use it.
    if pot[n1 - 1] != 0.0 {
        let xm = -pot[n1] / pot[n1 - 1];
        if xm < x {
            x = xm;
        }
    }

    // Chop the interval (0,x) until f <= 0.
    let mut f;
    loop {
        let xm = x * 0.1;
        f = pot[0];
        for i in 1..n {
            f = f * xm + pot[i];
        }
        if f <= 0.0 {
            break;
        }
        x = xm;
    }

    let mut dx = x;

    // Newton iteration until x converges to two decimal places.
    while (dx / x).abs() > 0.005 {
        q[0] = pot[0];
        for i in 1..n {
            q[i] = q[i - 1] * x + pot[i];
        }
        let f = q[n1];
        let mut delf = q[0];
        for i in 1..n1 {
            delf = delf * x + q[i];
        }
        dx = f / delf;
        x -= dx;
    }
    x
}

/// Returns a scale factor to multiply the coefficients of the polynomial,
/// to avoid overflow and undetected underflow interfering with the
/// convergence criterion. The factor is a power of `base`.
// R: complex.c:cpoly_scale
fn cpoly_scale(n: usize, pot: &[f64], eps: f64, big: f64, small: f64, base: f64) -> f64 {
    // Find the largest and smallest moduli of the coefficients.
    let high = big.sqrt();
    let lo = small / eps;
    let mut max_ = 0.0;
    let mut min_ = big;
    for &x in pot.iter().take(n) {
        if x > max_ {
            max_ = x;
        }
        if x != 0.0 && x < min_ {
            min_ = x;
        }
    }

    // Scale only if there are very large or very small components.
    if min_ < lo || max_ > high {
        let x = lo / min_;
        let mut sc = if x <= 1.0 {
            1.0 / (max_.sqrt() * min_.sqrt())
        } else {
            let sc = x;
            if big / sc > max_ { 1.0 } else { sc }
        };
        let ell = (sc.ln() / base.ln() + 0.5) as i32;
        sc = robigo_numerics::r_pow_di(base, ell);
        sc
    } else {
        1.0
    }
}

/// Complex division `c = a/b` that avoids overflow.
// R: complex.c:cdivid
fn cdivid(ar: f64, ai: f64, br: f64, bi: f64) -> (f64, f64) {
    if br == 0.0 && bi == 0.0 {
        // Division by zero: c = infinity.
        (f64::INFINITY, f64::INFINITY)
    } else if br.abs() >= bi.abs() {
        let r = bi / br;
        let d = br + r * bi;
        ((ar + ai * r) / d, (ai - ar * r) / d)
    } else {
        let r = br / bi;
        let d = bi + r * br;
        ((ar * r + ai) / d, (ai * r - ar) / d)
    }
}
