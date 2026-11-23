/* A connection's state as package C sees it (the A5 note's decision C):
 * R_GetConnection's struct Rconn is the connection's own state, so C reads
 * R's flags there and R sees what C writes. */
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Connections.h>
#include <errno.h>

#if R_CONNECTIONS_VERSION != 1
#error "this code expects R_CONNECTIONS_VERSION 1"
#endif

/* what C reads: isopen, canread, canwrite, text, mode, class, description */
SEXP rt_conn_state(SEXP sconn)
{
    Rconnection con = R_GetConnection(sconn);
    SEXP ans = PROTECT(allocVector(STRSXP, 7));
    SET_STRING_ELT(ans, 0, mkChar(con->isopen ? "isopen" : "closed"));
    SET_STRING_ELT(ans, 1, mkChar(con->canread ? "canread" : "-"));
    SET_STRING_ELT(ans, 2, mkChar(con->canwrite ? "canwrite" : "-"));
    SET_STRING_ELT(ans, 3, mkChar(con->text ? "text" : "binary"));
    SET_STRING_ELT(ans, 4, mkChar(con->mode));
    SET_STRING_ELT(ans, 5, mkChar(con->class));
    SET_STRING_ELT(ans, 6, mkChar(con->description));
    UNPROTECT(1);
    return ans;
}

/* the isopen flag written from C */
SEXP rt_conn_mark_closed(SEXP sconn)
{
    Rconnection con = R_GetConnection(sconn);
    con->isopen = FALSE;
    return R_NilValue;
}

/* con->read(buf, 1, n, con) with errno cleared first: the items read and
 * the errno the call left */
SEXP rt_conn_read(SEXP sconn, SEXP n)
{
    Rconnection con = R_GetConnection(sconn);
    unsigned char buf[16];
    int k = asInteger(n);
    if (k < 0 || k > 16) error("n out of range");
    errno = 0;
    size_t r = con->read(buf, 1, (size_t) k, con);
    int e = errno;
    SEXP ans = allocVector(INTSXP, 2);
    INTEGER(ans)[0] = (int) r;
    INTEGER(ans)[1] = e;
    return ans;
}

/* con->write(RAW(x), 1, length(x), con) with errno cleared first: the
 * items written and the errno the call left */
SEXP rt_conn_write(SEXP sconn, SEXP x)
{
    Rconnection con = R_GetConnection(sconn);
    if (TYPEOF(x) != RAWSXP) error("x must be a raw vector");
    errno = 0;
    size_t r = con->write(RAW(x), 1, (size_t) XLENGTH(x), con);
    int e = errno;
    SEXP ans = allocVector(INTSXP, 2);
    INTEGER(ans)[0] = (int) r;
    INTEGER(ans)[1] = e;
    return ans;
}
