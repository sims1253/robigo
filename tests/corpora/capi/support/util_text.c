/* util.c's text helpers that libR exports and grDevices' cairo module
 * imports when it is built without pango (cairoFns.c's FreeType path):
 * Rstrdup and Rf_utf8ToLatin1AdobeSymbol2utf8. */
#include <stdlib.h>
#include <R.h>
#include <Rinternals.h>

char *Rstrdup(const char *s);
const char *Rf_utf8ToLatin1AdobeSymbol2utf8(const char *in, Rboolean usePUA);

SEXP capi_rstrdup(SEXP x) {
    char *c = Rstrdup(CHAR(STRING_ELT(x, 0)));
    SEXP r = PROTECT(mkString(c));
    free(c);
    UNPROTECT(1);
    return r;
}

/* Latin-1 bytes read as Adobe Symbol codes, as UTF-8; with or without
 * the private-use area. */
SEXP capi_symbol_utf8(SEXP x, SEXP pua) {
    const char *s = Rf_utf8ToLatin1AdobeSymbol2utf8(translateCharUTF8(STRING_ELT(x, 0)), asLogical(pua));
    return ScalarString(mkCharCE(s, CE_UTF8));
}
