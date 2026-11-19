/* R_alloc from package C: an empty request is NULL (GNU R: "One programmer
 * has relied on this"), a block is C's to write until the .Call returns,
 * and a request past R_XLEN_T_MAX is an error. */
#include <string.h>
#include <R.h>
#include <Rinternals.h>

/* whether R_alloc(nelem, eltsize) gave NULL */
SEXP rt_r_alloc_null(SEXP nelem, SEXP eltsize)
{
    char *p = R_alloc((size_t) asReal(nelem), asInteger(eltsize));
    return ScalarLogical(p == NULL);
}

/* n bytes of a block, written by C and read back */
SEXP rt_r_alloc_fill(SEXP sn)
{
    int n = asInteger(sn);
    unsigned char *p = (unsigned char *) R_alloc((size_t) n, 1);
    for (int i = 0; i < n; i++)
        p[i] = (unsigned char) ((unsigned) i * 7u + 1u); /* (no signed overflow for a large n) */
    SEXP ans = PROTECT(allocVector(RAWSXP, n));
    /* R_alloc(0, 1) is NULL, and memcpy's source must be valid even for 0 */
    if (n > 0)
        memcpy(RAW(ans), p, (size_t) n);
    UNPROTECT(1);
    return ans;
}

/* 2^53 bytes: past R_XLEN_T_MAX (2^52) */
SEXP rt_r_alloc_huge(void)
{
    R_alloc((size_t) 1 << 53, 1);
    return R_NilValue;
}
