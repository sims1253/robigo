// R's Fortran (and Fortran-callable) routines: LINPACK QR and Cholesky,
// the optimizers and quadpack — what packages call via .Fortran or
// F77_CALL.
// R: appl/dqrdc2.f, dqrsl.f, dqrdc.f, dpofa.f, appl/uncmin.c,
//     appl/bfgs.c (nmmin), main/xxxpr.f
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Applic.h>
#include <R_ext/RS.h>

// Not declared in Applic.h.
void F77_NAME(dqrsl)(double *x, int *ldx, int *n, int *k, double *qraux,
                     double *y, double *qy, double *qty, double *b,
                     double *rsd, double *xb, int *job, int *info);
void F77_NAME(dqrdc)(double *x, int *ldx, int *n, int *p, double *qraux,
                     int *jpvt, double *work, int *job);
void F77_NAME(dpofa)(double *a, int *lda, int *n, int *info);
void F77_NAME(intpr)(const char *label, int *nchar, int *data, int *ndata,
                     size_t label_len);

// dqrdc2: the QR behind R's qr(); then dqrsl solves the least squares
// problem. A is 3x2 (column major), y = 3-vector.
SEXP capi_qr(void) {
    int n = 3, p = 2, ldx = 3;
    double x[6] = {2.0, 1.0, 1.0,  /* col 1 */
                   1.0, 3.0, 1.0}; /* col 2 */
    double tol = 1e-7, qraux[2], work[4];
    int pivot[2] = {1, 2}, rank = 0;
    F77_CALL(dqrdc2)(x, &ldx, &n, &p, &tol, &rank, qraux, pivot, work);

    double y[3] = {1.0, 2.0, 3.0}, qty[3], b[2], rsd[3], xb[3];
    int job = 110, info = 0;
    F77_CALL(dqrsl)(x, &ldx, &n, &p, qraux, y, NULL, qty, b, rsd, xb,
                    &job, &info);

    // dqrdc: the older non-pivoting Householder QR.
    double x2[6] = {2.0, 1.0, 1.0, 1.0, 3.0, 1.0};
    double qraux2[2], work2[2];
    int jpvt2[2] = {0, 0}, job0 = 0;
    F77_CALL(dqrdc)(x2, &ldx, &n, &p, qraux2, jpvt2, work2, &job0);

    // dpofa: Cholesky of a 2x2 SPD matrix.
    double a[4] = {4.0, 2.0, 2.0, 3.0};
    int lda = 2, n2 = 2, cinfo = 0;
    F77_CALL(dpofa)(a, &lda, &n2, &cinfo);

    SEXP ans = PROTECT(allocVector(REALSXP, 10));
    REAL(ans)[0] = rank;
    REAL(ans)[1] = qraux[0];
    REAL(ans)[2] = b[0];
    REAL(ans)[3] = b[1];
    REAL(ans)[4] = rsd[0] + rsd[1] + rsd[2];
    REAL(ans)[5] = info;
    REAL(ans)[6] = qraux2[0];
    REAL(ans)[7] = cinfo;
    REAL(ans)[8] = a[2];  // lower triangle entry of the factor
    REAL(ans)[9] = pivot[0] * 10 + pivot[1];
    UNPROTECT(1);
    return ans;
}

// The optimizers, on f(x, y) = (x-3)^2 + (y+1)^2.
static double capi_objfn(int n, double *par, void *ex) {
    double dx = par[0] - 3.0, dy = par[1] + 1.0;
    return dx * dx + dy * dy;
}

static void capi_objgr(int n, double *par, double *gr, void *ex) {
    gr[0] = 2.0 * (par[0] - 3.0);
    gr[1] = 2.0 * (par[1] + 1.0);
}

static void capi_fcn(int n, double *x, double *f, void *state) {
    f[0] = capi_objfn(n, x, state);
}

SEXP capi_optim(void) {
    // vmmin: BFGS.
    double b[2] = {0.0, 0.0}, fmin = 0.0;
    int mask[2] = {1, 1}, fncount = 0, grcount = 0, fail = -1;
    vmmin(2, b, &fmin, capi_objfn, capi_objgr, 100, 0, mask, 1e-8, 1e-8, 1,
          NULL, &fncount, &grcount, &fail);

    // nmmin: Nelder-Mead.
    double bvec[2] = {0.0, 0.5}, x[2], fmin2 = 0.0;
    int fail2 = 0, fncount2 = 0;
    nmmin(2, bvec, x, &fmin2, capi_objfn, &fail2, 1e-8, 1e-8, NULL,
          1.0, 0.5, 2.0, 0, &fncount2, 500);

    // optif9: uncmin (msg bit flags: 9 is minimal; dlt 1.0).
    double xs[2] = {0.0, 0.0}, typsiz[2] = {1.0, 1.0};
    double xpls[2], fpls = 0.0, gpls[2];
    double a[4], wrk[16];
    int msg = 9, itrmcd = 0, itncnt = 0;
    optif9(2, 2, xs, capi_fcn, capi_objgr, NULL, NULL, typsiz, 1.0, 1, 1,
           &msg, 2, 100, 1, 0, 1.0, 1e-8, 1.0, 1e-8, xpls, &fpls, gpls,
           &itrmcd, a, wrk, &itncnt);

    // Rdqags: quadpack on exp over [0, 1].
    SEXP ans = PROTECT(allocVector(REALSXP, 12));
    REAL(ans)[0] = b[0];
    REAL(ans)[1] = b[1];
    REAL(ans)[2] = fmin;
    REAL(ans)[3] = fail;
    REAL(ans)[4] = x[0];
    REAL(ans)[5] = fmin2;
    REAL(ans)[6] = fail2;
    REAL(ans)[7] = xpls[0];
    REAL(ans)[8] = xpls[1];
    REAL(ans)[9] = fpls;
    // allocVector does not initialize: the last two elements were printed
    // from whatever memory the allocator handed out (zeros in GNU R's
    // recorded run, which these keep).
    REAL(ans)[10] = 0.0;
    REAL(ans)[11] = 0.0;
    UNPROTECT(1);
    return ans;
}

// intpr: Fortran's integer printer (used via .Fortran).
SEXP capi_intpr(void) {
    int data[3] = {4, 5, 6};
    int ndata = 3, nchar = 3;
    F77_CALL(intpr)("xyz", &nchar, data, &ndata, (size_t)3);
    return R_NilValue;
}

// Rdqags: quadpack's QAGS on exp over [0, 1] (vectorized integrand).
static void capi_integrand(double *x, int n, void *ex) {
    for (int i = 0; i < n; i++) x[i] = exp(x[i]);
}

SEXP capi_dqags(void) {
    double a = 0.0, b = 1.0, epsabs = 1e-10, epsrel = 1e-10;
    double result = 0.0, abserr = 0.0;
    int neval = 0, ier = 0, limit = 100, lenw = 400, last = 0;
    int iwork[100];
    double work[400];
    Rdqags(capi_integrand, NULL, &a, &b, &epsabs, &epsrel, &result, &abserr,
           &neval, &ier, &limit, &lenw, &last, iwork, work);
    return ScalarReal(result + abserr);
}
