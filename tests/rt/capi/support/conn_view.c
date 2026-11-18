/* A connection's state as package C sees it (the A5 note's decision C):
 * R_GetConnection's struct Rconn is the connection's own state, so C reads
 * R's flags there and R sees what C writes. */
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Connections.h>

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
