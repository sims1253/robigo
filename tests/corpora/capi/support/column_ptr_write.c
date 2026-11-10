/* C reverses the columns of a list in place through the pointer
 * DATAPTR_RO gave, as data.table's setorder() does (reorder.c:
 * memcpy((char *)DATAPTR_RO(v) + ..., TMP, ...)): the columns are not the
 * call's arguments, and no setter reports the write. GNU R's columns are
 * the objects C wrote into, so R sees the new order in every column. */
#include <R.h>
#include <Rinternals.h>
#include <string.h>

static void reverse(void *p, size_t size, R_xlen_t n) {
    if (n < 2)
        return;
    char tmp[16], *a = (char *) p, *b = a + size * (n - 1);
    for (; a < b; a += size, b -= size) {
        memcpy(tmp, a, size);
        memcpy(a, b, size);
        memcpy(b, tmp, size);
    }
}

SEXP capi_reverse_columns(SEXP x) {
    if (TYPEOF(x) != VECSXP)
        error("capi_reverse_columns: need a list");
    for (R_xlen_t j = 0; j < XLENGTH(x); j++) {
        SEXP v = VECTOR_ELT(x, j);
        size_t size;
        switch (TYPEOF(v)) {
        case INTSXP: size = sizeof(int); break;
        case REALSXP: size = sizeof(double); break;
        case STRSXP: case VECSXP: size = sizeof(SEXP); break;
        default: error("capi_reverse_columns: unsupported column type");
        }
        /* deliberate: a write through DATAPTR_RO's const pointer */
        reverse((void *) DATAPTR_RO(v), size, XLENGTH(v));
    }
    return R_NilValue;
}
