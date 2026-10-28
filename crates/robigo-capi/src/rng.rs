//! RNG.c's API: GetRNGstate/PutRNGstate and the uniform, normal and
//! exponential generators, over the host interpreter's generator (the one
//! R-level runif() & co. use), so C and R draw from one stream.

use crate::host::op;
use robigo_numerics::rng::Rng;

/// Run `f` with the current interpreter's generator.
pub fn with_rng<T>(f: impl FnOnce(&mut Rng) -> T) -> T {
    crate::bridge::with_interp(|i| f(&mut i.rng))
}

// R: RNG.c:GetRNGstate — load .Random.seed
#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetRNGstate() {
    op("getRNGstate\0", &[]);
}

// R: RNG.c:PutRNGstate — store .Random.seed
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PutRNGstate() {
    op("putRNGstate\0", &[]);
}

#[unsafe(no_mangle)]
pub extern "C" fn unif_rand() -> f64 {
    with_rng(|r| r.unif_rand())
}

#[unsafe(no_mangle)]
pub extern "C" fn norm_rand() -> f64 {
    with_rng(|r| r.norm_rand())
}

#[unsafe(no_mangle)]
pub extern "C" fn exp_rand() -> f64 {
    with_rng(|r| r.exp_rand())
}

// R: arithmetic.c:R_pow, R_pow_di
#[unsafe(no_mangle)]
pub extern "C" fn R_pow(x: f64, y: f64) -> f64 {
    robigo_numerics::r_pow(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn R_pow_di(x: f64, n: std::ffi::c_int) -> f64 {
    robigo_numerics::r_pow_di(x, n)
}

// R: nmath/sign.c
#[unsafe(no_mangle)]
pub extern "C" fn Rf_sign(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 0.0 {
        1.0
    } else if x == 0.0 {
        0.0
    } else {
        -1.0
    }
}

// R: nmath/pgamma.c:log1mexp (R_Log1_Exp)
#[unsafe(no_mangle)]
pub extern "C" fn Rf_log1mexp(x: f64) -> f64 {
    robigo_numerics::r_log1_exp(x)
}

// R: nmath/rmultinom.c (the probability total in long double, as GNU R)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_rmultinom(
    n: std::ffi::c_int,
    prob: *const f64,
    k: std::ffi::c_int,
    rn: *mut std::ffi::c_int,
) {
    use robigo_numerics::ldouble::LDouble;
    let mut n = n;
    if k == i32::MIN || k < 1 {
        robigo_numerics::ml_warning(robigo_numerics::ME_DOMAIN, "rmultinom");
        return;
    }
    if n == i32::MIN || n < 0 {
        *rn = i32::MIN;
        return;
    }
    let k = k as usize;
    let prob = std::slice::from_raw_parts(prob, k);
    let rn = std::slice::from_raw_parts_mut(rn, k);
    let mut p_tot = LDouble::from_f64(0.0);
    for j in 0..k {
        let pp = prob[j];
        if !pp.is_finite() || !(0.0..=1.0).contains(&pp) {
            rn[j] = i32::MIN;
            return;
        }
        p_tot += pp;
        rn[j] = 0;
    }
    if (p_tot.to_f64() - 1.0).abs() > 1e-7 {
        crate::errors::error_str_owned(format!(
            "rbinom: probability sum should be 1, but is {}",
            crate::util::fmt_g(p_tot.to_f64())
        ))
    }
    if n == 0 || (k == 1 && p_tot.to_f64() == 0.0) {
        return;
    }
    for j in 0..k - 1 {
        if prob[j] != 0.0 {
            let pp = (LDouble::from_f64(prob[j]) / p_tot).to_f64();
            rn[j] = if pp < 1.0 { with_rng(|r| robigo_numerics::rand::rbinom(r, n as f64, pp)) as i32 } else { n };
            n -= rn[j];
        } else {
            rn[j] = 0;
        }
        if n <= 0 {
            return;
        }
        p_tot -= prob[j];
    }
    rn[k - 1] = n;
}
