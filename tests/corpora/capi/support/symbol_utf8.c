// Adobe Symbol text re-encoded to UTF-8: what the graphics engine does for
// plotmath's Greek letters on a device that wants UTF-8 symbols.
// R: main/sysutils.c:reEncode (CE_SYMBOL -> CE_UTF8 only)
#include <R.h>
#include <Rinternals.h>

SEXP capi_symbol_utf8(SEXP s, SEXP to) {
    int n = LENGTH(s);
    cetype_t out = (cetype_t) asInteger(to);
    SEXP ans = PROTECT(allocVector(STRSXP, n));
    for (int i = 0; i < n; i++) {
        const char *r = reEnc(CHAR(STRING_ELT(s, i)), CE_SYMBOL, out, 0);
        SET_STRING_ELT(ans, i, mkCharCE(r, out == CE_UTF8 ? CE_UTF8 : CE_NATIVE));
    }
    UNPROTECT(1);
    return ans;
}
