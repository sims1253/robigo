// Rmath: the d/p/q/r functions from package C code, the RNG from R's
// global state, and the small arithmetic helpers.
// R: include/Rmath.h, main/arithmetic.c:R_pow
#include <R.h>
#include <Rmath.h>
#include <Rinternals.h>

// Exported by libR but declared only for C++ in R_ext/Arith.h.
int R_isnancpp(double);

// The RNG: GetRNGstate/PutRNGstate bracket the direct generators.
// R's set.seed(42) in the driver makes these deterministic.
SEXP capi_rng(void) {
    GetRNGstate();
    double u = unif_rand();
    double n = norm_rand();
    double e = exp_rand();
    PutRNGstate();
    SEXP ans = PROTECT(allocVector(REALSXP, 3));
    REAL(ans)[0] = u;
    REAL(ans)[1] = n;
    REAL(ans)[2] = e;
    UNPROTECT(1);
    return ans;
}

// The Rf_ distributions callable from package code.
SEXP capi_rdist(void) {
    GetRNGstate();
    SEXP ans = PROTECT(allocVector(REALSXP, 4));
    REAL(ans)[0] = runif(1.5, 2.5);
    REAL(ans)[1] = rnorm(10.0, 0.5);
    REAL(ans)[2] = rpois(3.0);
    REAL(ans)[3] = rbeta(2.0, 3.0);
    PutRNGstate();
    UNPROTECT(1);
    return ans;
}

// Density, distribution and quantile functions; the arithmetic helpers;
// R_isnancpp.
SEXP capi_math(void) {
    SEXP ans = PROTECT(allocVector(REALSXP, 14));
    REAL(ans)[0] = dnorm4(0.0, 0.0, 1.0, 0);
    REAL(ans)[1] = pnorm5(1.959963984540054, 0.0, 1.0, 1, 0);
    REAL(ans)[2] = qnorm5(0.975, 0.0, 1.0, 1, 0);
    REAL(ans)[3] = pgamma(2.0, 1.5, 2.0, 1, 0);
    REAL(ans)[4] = gammafn(5.0);
    REAL(ans)[5] = lgammafn(100.0);
    REAL(ans)[6] = fmax2(1.5, 2.5);
    REAL(ans)[7] = fmin2(-1.5, 2.5);
    REAL(ans)[8] = imax2(3, 7);
    REAL(ans)[9] = imin2(3, 7);
    REAL(ans)[10] = R_pow(2.0, 10.0);
    REAL(ans)[11] = R_pow_di(2.0, 10);
    REAL(ans)[12] = R_isnancpp(R_NaN) + 2.0 * R_isnancpp(1.0);
    REAL(ans)[13] = fmax2(NA_REAL, 2.0);  // NA-aware max
    UNPROTECT(1);
    return ans;
}

// The wider d/p/q family, all deterministic (no RNG).
SEXP capi_dpq(void) {
    SEXP a = PROTECT(allocVector(REALSXP, 33));
    REAL(a)[0] = dbinom(3.0, 10.0, 0.4, 0);
    REAL(a)[1] = dnbinom(2.0, 5.0, 0.3, 0);
    REAL(a)[2] = pnbinom(2.0, 5.0, 0.3, 1, 0);
    REAL(a)[3] = pbeta(0.4, 2.0, 3.0, 1, 0);
    REAL(a)[4] = lbeta(2.5, 3.5);
    REAL(a)[5] = dpois(2.0, 3.0, 0);
    REAL(a)[6] = pbinom(3.0, 10.0, 0.4, 1, 0);
    REAL(a)[7] = ppois(2.0, 3.0, 1, 0);
    REAL(a)[8] = pt(1.5, 8.0, 1, 0);
    REAL(a)[9] = qbeta(0.5, 2.0, 3.0, 1, 0);
    REAL(a)[10] = qbinom(0.5, 10.0, 0.4, 1, 0);
    REAL(a)[11] = qchisq(0.5, 4.0, 1, 0);
    REAL(a)[12] = qnbinom(0.5, 5.0, 0.3, 1, 0);
    REAL(a)[13] = qpois(0.5, 3.0, 1, 0);
    REAL(a)[14] = dbeta(0.4, 2.0, 3.0, 0);
    REAL(a)[15] = dgamma(1.5, 2.0, 3.0, 0);
    REAL(a)[16] = dgeom(2.0, 0.3, 0);
    REAL(a)[17] = dlnorm(0.5, 0.0, 1.0, 0);
    REAL(a)[18] = dnbinom_mu(2.0, 5.0, 10.0, 0);
    REAL(a)[19] = dt(1.5, 8.0, 0);
    REAL(a)[20] = dunif(0.4, 0.0, 1.0, 0);
    REAL(a)[21] = dweibull(1.2, 2.0, 3.0, 0);
    REAL(a)[22] = choose(10.0, 3.0);
    REAL(a)[23] = lchoose(10.0, 3.0);
    REAL(a)[24] = fround(3.14159265358979, 4);
    REAL(a)[25] = sign(-2.5);
    REAL(a)[26] = bessel_k(1.5, 2.0, 1.0);
    REAL(a)[27] = pchisq(2.0, 4.0, 1, 0);
    REAL(a)[28] = pexp(0.5, 2.0, 1, 0);
    REAL(a)[29] = pgeom(2.0, 0.3, 1, 0);
    REAL(a)[30] = plogis(0.7, 0.0, 1.0, 1, 0);
    REAL(a)[31] = punif(0.4, 0.0, 1.0, 1, 0);
    REAL(a)[32] = pweibull(1.2, 2.0, 3.0, 1, 0);
    UNPROTECT(1);
    return a;
}

// The q/r tail (qgamma, qgeom, qt, rcauchy, ...) — the random ones are
// seeded from R.
SEXP capi_qr_more(void) {
    GetRNGstate();
    SEXP a = PROTECT(allocVector(REALSXP, 12));
    REAL(a)[0] = qgamma(0.5, 2.0, 3.0, 1, 0);
    REAL(a)[1] = qgeom(0.5, 0.3, 1, 0);
    REAL(a)[2] = qt(0.9, 8.0, 1, 0);
    REAL(a)[3] = rcauchy(1.0, 0.5);
    REAL(a)[4] = rchisq(4.0);
    REAL(a)[5] = rexp(2.0);
    REAL(a)[6] = rhyper(10.0, 7.0, 3.0);
    REAL(a)[7] = rt(5.0);
    REAL(a)[8] = rbinom(10.0, 0.4);
    REAL(a)[9] = rgamma(2.0, 3.0);
    REAL(a)[10] = rgeom(0.3);
    REAL(a)[11] = rnbinom(5.0, 0.3);
    PutRNGstate();
    UNPROTECT(1);
    return a;
}
