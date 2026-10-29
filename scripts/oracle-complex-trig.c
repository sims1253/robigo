/* Records the bits of the complex Math-group kernels R 4.5.0 uses:
 * glibc's ccos/csin/ctan/ccosh/csinh/ctanh/casin/cacos/catan and R's
 * z_* wrappers (complex.c), over the special-value grid and random bit
 * patterns.  Run on the CI image's glibc (robigo-oracle:4.5.0).
 */
#define _GNU_SOURCE
#include <complex.h>
#include <math.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

/* R_FINITE (Rinternals.h): not NaN and not Inf */
#define R_FINITE(X) (R_IsNan(X) == 0 && !IS_INF(X))
static int R_IsNan(double x) { return x != x; }
static int IS_INF(double x) { return x == 1.0/0.0 || x == -1.0/0.0; }

static double complex z_tan(double complex z)
{
    double y = cimag(z);
    double complex r = ctan(z);
    if(R_FINITE(y) && fabs(y) > 25.0) {
#if __GNUC__
	__imag__ r = y < 0 ? -1.0 : 1.0;
#else
	r = creal(r) + (y < 0 ? -1.0 : 1.0) * I;
#endif
    }
    return r;
}

static double complex z_asin(double complex z)
{
    if(cimag(z) == 0 && fabs(creal(z)) > 1) {
	double alpha, t1, t2, x = creal(z), ri;
	t1 = 0.5 * fabs(x + 1);
	t2 = 0.5 * fabs(x - 1);
	alpha = t1 + t2;
	ri = log(alpha + sqrt(alpha*alpha - 1));
	if(x > 1) ri *= -1;
	return asin(t1  - t2) + ri*I;
    }
    return casin(z);
}

static double complex z_acos(double complex z)
{
    if(cimag(z) == 0 && fabs(creal(z)) > 1) return M_PI_2 - z_asin(z);
    return cacos(z);
}

static double complex z_atan(double complex z)
{
    if(creal(z) == 0 && fabs(cimag(z)) > 1) {
	double y = cimag(z), rr, ri;
	rr = (y > 0) ? M_PI_2 : -M_PI_2;
	ri = 0.25 * log(((y + 1) * (y + 1))/((y - 1) * (y - 1)));
	return rr + ri*I;
    }
    return catan(z);
}

static double complex z_acosh(double complex z)
{
    return z_acos(z) * I;
}

static double complex z_asinh(double complex z)
{
    return -I * z_asin(z * I);
}

static double complex z_atanh(double complex z)
{
    return -I * z_atan(z * I);
}

typedef double complex (*cfun)(double complex);
static const char *names[] = {
    "ccos", "csin", "ctan", "ccosh", "csinh", "ctanh",
    "casin", "cacos", "catan",
    "ztan", "zasin", "zacos", "zatan", "zacosh", "zasinh", "zatanh",
};
static const cfun fns[] = {
    ccos, csin, ctan, ccosh, csinh, ctanh,
    casin, cacos, catan,
    z_tan, z_asin, z_acos, z_atan, z_acosh, z_asinh, z_atanh,
};

static uint64_t bits(double x) { uint64_t u; __builtin_memcpy(&u, &x, 8); return u; }
static double from_bits(uint64_t u) { double x; __builtin_memcpy(&x, &u, 8); return x; }

static uint64_t sm64(uint64_t *s){uint64_t z=(*s+=0x9E3779B97F4A7C15ULL);z=(z^(z>>30))*0xBF58476D1CE4E5B9ULL;z=(z^(z>>27))*0x94D049BB133111EBULL;return z^(z>>31);}

static void emit(const char *name, double complex z, double complex r)
{
    printf("%s %016lx %016lx 0000000000000000 0000000000000000 %016lx %016lx\n",
           name,
           (unsigned long)bits(creal(z)), (unsigned long)bits(cimag(z)),
           (unsigned long)bits(creal(r)), (unsigned long)bits(cimag(r)));
}

int main(void)
{
    static const char *comp[] = {
        "7ff00000000007a2", /* NA_real_ (signalling) */
        "7ff80000000007a2", /* NA, quiet */
        "fff8000000000000", /* 0/0 */
        "7ff8000000000000", /* -(0/0) */
        "7ff8000000000123", /* payload */
        "fff4000000000456", /* signalling payload, negative */
        "7ff0000000000000", "fff0000000000000", /* +-Inf */
        "0000000000000000", "8000000000000000", /* +-0 */
        "3ff0000000000000", "c008000000000000", /* 1, -3 */
        "7fefffffffffffff", "0000000000000001", /* max, min subnormal */
    };
    const int ncomp = sizeof(comp) / sizeof(comp[0]);
    int n = ncomp * ncomp;
    double *zr = malloc(n * sizeof(double)), *zi = malloc(n * sizeof(double));
    for (int i = 0; i < ncomp; i++)
        for (int k = 0; k < ncomp; k++) {
            zr[i * ncomp + k] = from_bits(strtoull(comp[i], NULL, 16));
            zi[i * ncomp + k] = from_bits(strtoull(comp[k], NULL, 16));
        }
    /* Targeted extra values: branch-cut and reduction thresholds of the
     * templates. */
    static const double extra[] = {
        0.5, -0.5, 0.75, -0.75, 1.0, -1.0, 1.5, -1.5, 2.0, -2.0,
        2.2250738585072014e-308 /* DBL_MIN */, -2.2250738585072014e-308,
        2.220446049250313e-16 /* DBL_EPSILON */, -2.220446049250313e-16,
        4.930380657631324e-32 /* EPSILON^2 */, -4.930380657631324e-32,
        2.775557561562891e-17 /* EPSILON/8 */, -2.775557561562891e-17,
        4503599627370496.0 /* 1/EPS */, -4503599627370496.0,
        72057594037927936.0 /* 16/EPS */, -72057594037927936.0,
        25.0, -25.0, 25.1, -25.1, 26.0, -26.0,
        709.0, -709.0, 709.78, -709.78, 710.0, -710.0, 1060.0, -1060.0,
        354.0, -354.0, 355.0, -355.0,
        0.6931471805599453, -0.6931471805599453,
        1e300, -1e300, 1e-300, -1e-300, 3.0, -3.0, 4.0, -4.0,
        0.1, -0.1, 0.9, -0.9, 1.1, -1.1, 1.4, -1.4, 1.6, -1.6,
        1.0000000000000002, 0.9999999999999999,
    };
    const int nextra = sizeof(extra) / sizeof(extra[0]);
    int m = n + nextra * nextra;
    zr = realloc(zr, m * sizeof(double));
    zi = realloc(zi, m * sizeof(double));
    for (int i = 0; i < nextra; i++)
        for (int k = 0; k < nextra; k++) {
            zr[n + i * nextra + k] = extra[i];
            zi[n + i * nextra + k] = extra[k];
        }
    /* Random bit patterns. */
    int nrand = 20000;
    uint64_t s = 20261029;
    zr = realloc(zr, (m + nrand) * sizeof(double));
    zi = realloc(zi, (m + nrand) * sizeof(double));
    for (int i = 0; i < nrand; i++) {
        zr[m + i] = from_bits(sm64(&s));
        zi[m + i] = from_bits(sm64(&s));
    }
    int total = m + nrand;
    for (int i = 0; i < total; i++) {
        double complex z = zr[i] + zi[i] * I;
        for (int f = 0; f < (int)(sizeof(fns) / sizeof(fns[0])); f++)
            emit(names[f], z, fns[f](z));
    }
    fprintf(stderr, "inputs %d, lines %d\n", total,
            total * (int)(sizeof(fns) / sizeof(fns[0])));
    return 0;
}
