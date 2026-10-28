//! Tukey's studentized range distribution: ptukey.c, qtukey.c.

use super::gamma::lgammafn;
use super::ldouble::LDouble;
use super::norm::pnorm;
use super::*;

/// R: nmath/ptukey.c:wprob
fn wprob(w: f64, rr: f64, cc: f64) -> f64 {
    const NLEG: usize = 12;
    const IHALF: usize = 6;
    const C1: f64 = -30.0;
    const C2: f64 = -50.0;
    const C3: f64 = 60.0;
    const BB: f64 = 8.0;
    const WLAR: f64 = 3.0;
    const WINCR1: f64 = 2.0;
    const WINCR2: f64 = 3.0;
    const XLEG: [f64; IHALF] = [
        0.981560634246719250690549090149,
        0.904117256370474856678465866119,
        0.769902674194304687036893833213,
        0.587317954286617447296702418941,
        0.367831498998180193752691536644,
        0.125233408511468915472441369464,
    ];
    const ALEG: [f64; IHALF] = [
        0.047175336386511827194615961485,
        0.106939325995318430960254718194,
        0.160078328543346226334652529543,
        0.203167426723065921749064455810,
        0.233492536538354808760849898925,
        0.249147045813402785000562436043,
    ];

    let qsqz = w * 0.5;

    // if w >= 16 then the integral lower bound (occurs for c=20) is
    // 0.99999999999995 so return a value of 1.
    if qsqz >= BB {
        return 1.0;
    }

    // find (f(w/2) - 1) ^ cc (first term in integral of Hartley's form).
    let mut pr_w = 2.0 * pnorm(qsqz, 0.0, 1.0, true, false) - 1.0;
    // if pr_w ^ cc < 2e-22 then set pr_w = 0
    if pr_w >= (C2 / cc).exp() {
        pr_w = pr_w.powf(cc);
    } else {
        pr_w = 0.0;
    }

    // if w is large then the second component of the integral is small,
    // so fewer intervals are needed.
    let wincr = if w > WLAR { WINCR1 } else { WINCR2 };

    let mut blb = LDouble::from_f64(qsqz);
    let binc = (BB - qsqz) / wincr;
    let mut bub = blb + binc;
    let mut einsum = LDouble::from_f64(0.0);

    // integrate over each interval
    let cc1 = cc - 1.0;
    let mut wi = 1.0;
    while wi <= wincr {
        let mut elsum = LDouble::from_f64(0.0);
        let a = (LDouble::from_f64(0.5) * (bub + blb)).to_f64();

        // legendre quadrature with order = nleg
        let b = (LDouble::from_f64(0.5) * (bub - blb)).to_f64();

        for jj in 1..=NLEG {
            let (j, xx) = if IHALF < jj { (NLEG - jj + 1, XLEG[NLEG - jj]) } else { (jj, -XLEG[jj - 1]) };
            let c = b * xx;
            let ac = a + c;

            // if exp(-qexpo/2) < 9e-14, then doesn't contribute to integral
            let qexpo = ac * ac;
            if qexpo > C3 {
                break;
            }

            let pplus = 2.0 * pnorm(ac, 0.0, 1.0, true, false);
            let pminus = 2.0 * pnorm(ac, w, 1.0, true, false);

            // if rinsum ^ (cc-1) < 9e-14, then doesn't contribute to integral
            let mut rinsum = (pplus * 0.5) - (pminus * 0.5);
            if rinsum >= (C1 / cc1).exp() {
                rinsum = (ALEG[j - 1] * (-(0.5 * qexpo)).exp()) * rinsum.powf(cc1);
                elsum += rinsum;
            }
        }
        elsum *= ((2.0 * b) * cc) * M_1_SQRT_2PI;
        einsum += elsum;
        blb = bub;
        bub += binc;
        wi += 1.0;
    }

    // if pr_w ^ rr < 9e-14, then return 0
    pr_w += einsum.to_f64();
    if pr_w <= (C1 / rr).exp() {
        return 0.0;
    }

    pr_w = pr_w.powf(rr);
    if pr_w >= 1.0 {
        // 1 was iMax was eps
        return 1.0;
    }
    pr_w
}

/// R: nmath/ptukey.c:ptukey
pub fn ptukey(q: f64, rr: f64, cc: f64, df: f64, lower_tail: bool, log_p: bool) -> f64 {
    const NLEGQ: usize = 16;
    const IHALFQ: usize = 8;
    const EPS1: f64 = -30.0;
    const EPS2: f64 = 1.0e-14;
    const DHAF: f64 = 100.0;
    const DQUAR: f64 = 800.0;
    const DEIGH: f64 = 5000.0;
    const DLARG: f64 = 25000.0;
    const ULEN1: f64 = 1.0;
    const ULEN2: f64 = 0.5;
    const ULEN3: f64 = 0.25;
    const ULEN4: f64 = 0.125;
    const XLEGQ: [f64; IHALFQ] = [
        0.989400934991649932596154173450,
        0.944575023073232576077988415535,
        0.865631202387831743880467897712,
        0.755404408355003033895101194847,
        0.617876244402643748446671764049,
        0.458016777657227386342419442984,
        0.281603550779258913230460501460,
        0.950125098376374401853193354250e-1,
    ];
    const ALEGQ: [f64; IHALFQ] = [
        0.271524594117540948517805724560e-1,
        0.622535239386478928628438369944e-1,
        0.951585116824927848099251076022e-1,
        0.124628971255533872052476282192,
        0.149595988816576732081501730547,
        0.169156519395002538189312079030,
        0.182603415044923588866763667969,
        0.189450610455068496285396723208,
    ];

    if q.is_nan() || rr.is_nan() || cc.is_nan() || df.is_nan() {
        return ML_NAN;
    }

    if q <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }

    // df must be > 1; there must be at least two values
    if df < 2.0 || rr < 1.0 || cc < 2.0 {
        return ML_NAN;
    }

    if !q.is_finite() {
        return r_dt_1(lower_tail, log_p);
    }

    if df > DLARG {
        return r_dt_val(wprob(q, rr, cc), lower_tail, log_p);
    }

    // calculate leading constant
    let f2 = df * 0.5;
    // lgammafn(u) = log(gamma(u))
    let mut f2lf = (f2 * df.ln() - df * M_LN2) - lgammafn(f2);
    let f21 = f2 - 1.0;

    // integral is divided into unit, half-unit, quarter-unit, or
    // eighth-unit length intervals depending on the value of the
    // degrees of freedom.
    let ff4 = df * 0.25;
    let ulen = if df <= DHAF {
        ULEN1
    } else if df <= DQUAR {
        ULEN2
    } else if df <= DEIGH {
        ULEN3
    } else {
        ULEN4
    };

    f2lf += ulen.ln();

    // integrate over each subinterval
    let mut ans = 0.0;
    let mut otsum = 0.0;

    for i in 1..=50 {
        otsum = 0.0;

        // legendre quadrature with order = nlegq
        // nodes (stored in xlegq) are symmetric around zero.
        let twa1 = (2.0 * i as f64 - 1.0) * ulen;

        for jj in 1..=NLEGQ {
            let (j, t1) = if IHALFQ < jj {
                let j = jj - IHALFQ - 1;
                let t1 = (f2lf + (f21 * (twa1 + (XLEGQ[j] * ulen)).ln())) - (((XLEGQ[j] * ulen) + twa1) * ff4);
                (j, t1)
            } else {
                let j = jj - 1;
                let t1 = (f2lf + (f21 * (twa1 - (XLEGQ[j] * ulen)).ln())) + (((XLEGQ[j] * ulen) - twa1) * ff4);
                (j, t1)
            };

            // if exp(t1) < 9e-14, then doesn't contribute to integral
            if t1 >= EPS1 {
                let qsqz = if IHALFQ < jj {
                    q * (((XLEGQ[j] * ulen) + twa1) * 0.5).sqrt()
                } else {
                    q * (((-(XLEGQ[j] * ulen)) + twa1) * 0.5).sqrt()
                };

                // call wprob to find integral of range portion
                let wprb = wprob(qsqz, rr, cc);
                let rotsum = (wprb * ALEGQ[j]) * t1.exp();
                otsum += rotsum;
            }
        }

        // if integral for interval i < 1e-14, then stop.  However, in
        // order to avoid small area under left tail, at least 1 / ulen
        // intervals are calculated.
        if i as f64 * ulen >= 1.0 && otsum <= EPS2 {
            break;
        }

        ans += otsum;
    }

    if otsum > EPS2 {
        // not converged
        ml_warning(ME_PRECISION, "ptukey");
    }
    if ans > 1.0 {
        ans = 1.0;
    }
    r_dt_val(ans, lower_tail, log_p)
}

/// R: nmath/qtukey.c:qinv
fn qinv(p: f64, c: f64, v: f64) -> f64 {
    const P0: f64 = 0.322232421088;
    const Q0: f64 = 0.993484626060e-01;
    const P1: f64 = -1.0;
    const Q1: f64 = 0.588581570495;
    const P2: f64 = -0.342242088547;
    const Q2: f64 = 0.531103462366;
    const P3: f64 = -0.204231210125;
    const Q3: f64 = 0.103537752850;
    const P4: f64 = -0.453642210148e-04;
    const Q4: f64 = 0.38560700634e-02;
    const C1: f64 = 0.8832;
    const C2: f64 = 0.2368;
    const C3: f64 = 1.214;
    const C4: f64 = 1.208;
    const C5: f64 = 1.4142;
    const VMAX: f64 = 120.0;

    let ps = 0.5 - 0.5 * p;
    let yi = (1.0 / (ps * ps)).ln().sqrt();
    let mut t =
        yi + ((((yi * P4 + P3) * yi + P2) * yi + P1) * yi + P0) / ((((yi * Q4 + Q3) * yi + Q2) * yi + Q1) * yi + Q0);
    if v < VMAX {
        t += (t * t * t + t) / v / 4.0;
    }
    let mut q = C1 - C2 * t;
    if v < VMAX {
        q += -C3 / v + C4 * t / v;
    }
    t * (q * (c - 1.0).ln() + C5)
}

/// R: nmath/qtukey.c:qtukey
#[allow(unused_assignments)] // `valx1 = -p` mirrors the C and is overwritten below
pub fn qtukey(p: f64, rr: f64, cc: f64, df: f64, lower_tail: bool, log_p: bool) -> f64 {
    const EPS: f64 = 0.0001;
    const MAXITER: i32 = 50;

    if p.is_nan() || rr.is_nan() || cc.is_nan() || df.is_nan() {
        ml_warning(ME_DOMAIN, "qtukey");
        return p + rr + cc + df;
    }

    // df must be > 1; there must be at least two values
    if df < 2.0 || rr < 1.0 || cc < 2.0 {
        return ML_NAN;
    }

    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }

    let p = r_dt_qiv(p, lower_tail, log_p); // lower_tail, non-log "p"

    // Initial value
    let mut x0 = qinv(p, cc, df);

    // Find prob(value < x0)
    let mut valx0 = ptukey(x0, rr, cc, df, true, false) - p;

    // Find the second iterate and prob(value < x1).  If the first
    // iterate has probability value exceeding p then second iterate is
    // 1 less than first iterate; otherwise it is 1 greater.
    let mut x1 = if valx0 > 0.0 { fmax2(0.0, x0 - 1.0) } else { x0 + 1.0 };
    let mut valx1 = ptukey(x1, rr, cc, df, true, false) - p;

    // Find new iterate
    let mut ans = 0.0;
    for _iter in 1..MAXITER {
        ans = x1 - ((valx1 * (x1 - x0)) / (valx1 - valx0));
        valx0 = valx1;

        // New iterate must be >= 0
        x0 = x1;
        if ans < 0.0 {
            ans = 0.0;
            valx1 = -p;
        }
        // Find prob(value < new iterate)
        valx1 = ptukey(ans, rr, cc, df, true, false) - p;
        x1 = ans;

        // If the difference between two successive iterates is less
        // than eps, stop
        let xabs = (x1 - x0).abs();
        if xabs < EPS {
            return ans;
        }
    }

    // The process did not converge in 'maxiter' iterations
    ml_warning(ME_NOCONV, "qtukey");
    ans
}
