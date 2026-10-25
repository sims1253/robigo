//! The F distribution: df.c, pf.c, qf.c.

use super::beta::{pbeta, qbeta};
use super::binom::dbinom_raw;
use super::pgamma::{dgamma, pchisq, qchisq};
use super::*;

/// R: nmath/df.c:df
pub fn df(x: f64, m: f64, n: f64, give_log: bool) -> f64 {
    if x.is_nan() || m.is_nan() || n.is_nan() {
        return x + m + n;
    }
    if m <= 0.0 || n <= 0.0 {
        return ML_NAN;
    }
    if x < 0.0 {
        return r_d__0(give_log);
    }
    if x == 0.0 {
        return if m > 2.0 {
            r_d__0(give_log)
        } else if m == 2.0 {
            r_d__1(give_log)
        } else {
            ML_POSINF
        };
    }
    if !m.is_finite() && !n.is_finite() {
        return if x == 1.0 { ML_POSINF } else { r_d__0(give_log) };
    }
    if !n.is_finite() {
        return dgamma(x, m / 2.0, 2.0 / m, give_log);
    }
    if m > 1e14 {
        let dens = dgamma(1.0 / x, n / 2.0, 2.0 / n, give_log);
        return if give_log { dens - 2.0 * x.ln() } else { dens / (x * x) };
    }
    let mut f = 1.0 / (n + x * m);
    let q = n * f;
    let p = x * m * f;
    let dens;
    if m >= 2.0 {
        f = m * q / 2.0;
        dens = dbinom_raw((m - 2.0) / 2.0, (m + n - 2.0) / 2.0, p, q, give_log);
    } else {
        f = m * m * q / (2.0 * p * (m + n));
        dens = dbinom_raw(m / 2.0, (m + n) / 2.0, p, q, give_log);
    }
    if give_log { f.ln() + dens } else { f * dens }
}

/// R: nmath/pf.c:pf
pub fn pf(x: f64, df1: f64, df2: f64, lower_tail: bool, log_p: bool) -> f64 {
    if x.is_nan() || df1.is_nan() || df2.is_nan() {
        return x + df2 + df1;
    }
    if df1 <= 0.0 || df2 <= 0.0 {
        return ML_NAN;
    }
    if x <= 0.0 {
        return r_dt_0(lower_tail, log_p);
    }
    if x >= ML_POSINF {
        return r_dt_1(lower_tail, log_p);
    }
    if df2 == ML_POSINF {
        if df1 == ML_POSINF {
            if x < 1.0 {
                return r_dt_0(lower_tail, log_p);
            }
            if x == 1.0 {
                return if log_p { -M_LN2 } else { 0.5 };
            }
            if x > 1.0 {
                return r_dt_1(lower_tail, log_p);
            }
        }
        return pchisq(x * df1, df1, lower_tail, log_p);
    }
    if df1 == ML_POSINF {
        return pchisq(df2 / x, df2, !lower_tail, log_p);
    }
    let x = if df1 * x > df2 {
        pbeta(df2 / (df2 + df1 * x), df2 / 2.0, df1 / 2.0, !lower_tail, log_p)
    } else {
        pbeta(df1 * x / (df2 + df1 * x), df1 / 2.0, df2 / 2.0, lower_tail, log_p)
    };
    if !x.is_nan() { x } else { ML_NAN }
}

/// R: nmath/qf.c:qf
pub fn qf(p: f64, df1: f64, df2: f64, lower_tail: bool, log_p: bool) -> f64 {
    if p.is_nan() || df1.is_nan() || df2.is_nan() {
        return p + df1 + df2;
    }
    if df1 <= 0.0 || df2 <= 0.0 {
        return ML_NAN;
    }
    if let Some(v) = r_q_p01_boundaries(p, 0.0, ML_POSINF, lower_tail, log_p) {
        return v;
    }
    if df1 <= df2 && df2 > 4e5 {
        if !df1.is_finite() {
            return 1.0;
        }
        return qchisq(p, df1, lower_tail, log_p) / df1;
    } else if df1 > 4e5 {
        return df2 / qchisq(p, df2, !lower_tail, log_p);
    }
    let p = (1.0 / qbeta(p, df2 / 2.0, df1 / 2.0, !lower_tail, log_p) - 1.0) * (df2 / df1);
    if !p.is_nan() { p } else { ML_NAN }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects mismatches so that one run reports every failing value.
    macro_rules! chk {
        ($bad:ident, $e:expr, $v:expr) => {{
            let got: f64 = $e;
            let want: f64 = $v;
            if !(got == want || (got.is_nan() && want.is_nan())) {
                $bad.push(format!("{} = {:?} (want {:?})", stringify!($e), got, want));
            }
        }};
    }

    // Reference values: GNU R 4.5.0, sprintf("%.17g", .Internal(df(...))) etc.
    #[test]
    fn df_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, df(1.5, 3.0, 7.0, false), 0.23397612413061702);
        chk!(bad, df(1.5, 3.0, 7.0, true), -1.4525362024600623);
        chk!(bad, df(0.0, 2.0, 5.0, false), 1.0);
        chk!(bad, df(0.0, 1.0, 5.0, false), f64::INFINITY);
        chk!(bad, df(0.5, 1.0, 1.0, false), 0.30010543871903533);
        chk!(bad, df(2.0, 1e+15, 7.0, false), 0.18535812300316512);
        chk!(bad, df(2.0, 5.0, f64::INFINITY, false), 0.14167277670867237);
        chk!(bad, df(1.0, f64::INFINITY, f64::INFINITY, false), f64::INFINITY);
        chk!(bad, df(3.0, 0.5, 2.5, true), -3.2418570643897198);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn pf_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, pf(2.5, 4.0, 12.0, true, false), 0.90183818882689537);
        chk!(bad, pf(2.5, 4.0, 12.0, false, false), 0.098161811173104588);
        chk!(bad, pf(2.5, 4.0, 12.0, false, true), -2.3211380275159561);
        chk!(bad, pf(3.0, f64::INFINITY, 10.0, true, false), 0.97245674321047137);
        chk!(bad, pf(0.7, 5.0, f64::INFINITY, true, false), 0.3766123722504181);
        chk!(bad, pf(1.0, f64::INFINITY, f64::INFINITY, true, false), 0.5);
        chk!(bad, pf(0.01, 3.0, 8.0, true, true), -6.5099714559516464);
        chk!(bad, pf(100.0, 1.0, 1.0, false, false), 0.06345103486110712);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }

    #[test]
    fn qf_values() {
        let mut bad: Vec<String> = Vec::new();
        chk!(bad, qf(0.95, 4.0, 12.0, true, false), 3.2591667269012485);
        chk!(bad, qf(0.05, 1.0, 1.0, true, false), 0.0061939586571082561);
        chk!(bad, qf(0.5, 5e+05, 10.0, true, false), 1.0704554778227708);
        chk!(bad, qf(0.5, 10.0, 5e+05, true, false), 0.93418177655919687);
        chk!(bad, qf(-3.0, 6.0, 9.0, false, true), 3.3791765805668894);
        chk!(bad, qf(0.999, 2.0, 3.0, true, false), 148.49999999999991);
        chk!(bad, qf(0.3, f64::INFINITY, f64::INFINITY, true, false), 1.0);
        assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    }
}
