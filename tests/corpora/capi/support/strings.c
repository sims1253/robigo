// CHARSXPs: construction, encodings, translation to the native charset.
// R: main/util.c:mkChar, main/character.c:translateChar
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Riconv.h>
#include <R_ext/GraphicsEngine.h>  // pulls in GraphicsDevice.h: Rf_ucstoutf8
#include <string.h>

// mkChar / mkString / R_CHAR / CHAR / mkCharLen / STRING_ELT. mkString
// returns a length-1 STRSXP, so it is returned as a list element.
SEXP capi_mk(SEXP s) {
    SEXP a = PROTECT(mkChar("plain"));
    SEXP b = PROTECT(mkCharLen(CHAR(STRING_ELT(s, 0)), 3));
    SEXP str = PROTECT(allocVector(STRSXP, 4));
    SET_STRING_ELT(str, 0, a);
    SET_STRING_ELT(str, 1, mkChar(R_CHAR(STRING_ELT(s, 1))));
    SET_STRING_ELT(str, 2, b);
    SET_STRING_ELT(str, 3, R_BlankString);
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, str);
    SET_VECTOR_ELT(ans, 1, mkString("mkString"));
    UNPROTECT(4);
    return ans;
}

// mkCharCE / mkCharLenCE / getCharCE: the four encodings.
// R: main/util.c:mkCharCE
SEXP capi_ce(void) {
    SEXP utf8 = PROTECT(mkCharCE("\xc3\xa9t\xc3\xa9", CE_UTF8));
    SEXP latin1 = PROTECT(mkCharCE("caf\xe9", CE_LATIN1));
    SEXP bytes = PROTECT(mkCharCE("a\xb0" "b", CE_BYTES));
    SEXP utf8len = PROTECT(mkCharLenCE("truncated\xc3\xa9", 9, CE_UTF8));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SEXP enc = PROTECT(allocVector(INTSXP, 4));
    INTEGER(enc)[0] = getCharCE(utf8);
    INTEGER(enc)[1] = getCharCE(latin1);
    INTEGER(enc)[2] = getCharCE(bytes);
    INTEGER(enc)[3] = getCharCE(mkChar("ascii"));
    SET_VECTOR_ELT(ans, 0, enc);
    SET_VECTOR_ELT(ans, 1, utf8len);
    UNPROTECT(6);
    return ans;
}

// translateChar / translateCharUTF8 re-encode into the current locale.
// R: main/character.c:translateChar
SEXP capi_translate(void) {
    SEXP latin1 = PROTECT(mkCharCE("caf\xe9", CE_LATIN1));
    SEXP utf8 = PROTECT(mkCharCE("\xc3\xa9t\xc3\xa9", CE_UTF8));
    const char *a = translateChar(latin1);
    const char *b = translateCharUTF8(utf8);
    SEXP ans = PROTECT(allocVector(STRSXP, 2));
    SET_STRING_ELT(ans, 0, mkCharCE(a, CE_UTF8));
    SET_STRING_ELT(ans, 1, mkCharCE(b, CE_UTF8));
    UNPROTECT(3);
    return ans;
}

// Rf_asChar: the first element of an atomic vector as a CHARSXP.
// R: main/coerce.c:asChar
SEXP capi_asChar(SEXP x) {
    return ScalarString(asChar(x));
}

// charIsASCII (on the CHARSXP) and R_nchar in its three modes.
// R: main/sysutils.c:R_nchar, main/util.c:Rf_charIsASCII
SEXP capi_ascii(SEXP x) {
    SEXP ans = PROTECT(allocVector(LGLSXP, LENGTH(x)));
    for (int i = 0; i < LENGTH(x); i++)
        LOGICAL(ans)[i] = charIsASCII(STRING_ELT(x, i));
    UNPROTECT(1);
    return ans;
}

// Re-encoding and locale facts: reEnc, ucstoutf8, Riconv, StringBlank,
// the locale flags libR exports.
// R: main/sysutils.c:reEnc, main/uncxxc? ucstoutf8, R_ext/Riconv.h
SEXP capi_reencode(void) {
    SEXP latin1 = PROTECT(mkCharCE("caf\xe9", CE_LATIN1));
    const char *to_utf8 = reEnc(CHAR(latin1), CE_LATIN1, CE_UTF8, 1);
    char buf[16];
    size_t n = Rf_ucstoutf8(buf, 0x00e9);  // one UCS-4 codepoint
    void *cd = Riconv_open("UTF-8", "ISO-8859-1");
    char inbuf[3] = {(char)0xe9, 0};
    char outbuf[8];
    char *pin = inbuf, *pout = outbuf;
    size_t inleft = 1, outleft = 8;
    size_t bad = Riconv(cd, (const char **)&pin, &inleft, &pout, &outleft);
    *pout = 0;
    Riconv_close(cd);
    extern Rboolean mbcslocale, utf8locale, known_to_be_latin1;
    SEXP ans = PROTECT(allocVector(VECSXP, 7));
    SET_VECTOR_ELT(ans, 0, mkCharCE(to_utf8, CE_UTF8));
    SET_VECTOR_ELT(ans, 1, mkCharLen(buf, (int)n));
    SET_VECTOR_ELT(ans, 2, ScalarInteger((int)bad));
    SET_VECTOR_ELT(ans, 3, mkChar(outbuf));
    SET_VECTOR_ELT(ans, 4, ScalarLogical(mbcslocale));
    SET_VECTOR_ELT(ans, 5, ScalarLogical(utf8locale));
    SET_VECTOR_ELT(ans, 6, ScalarLogical(StringBlank(R_BlankString)));
    UNPROTECT(2);
    return ans;
}

SEXP capi_nchar(SEXP s) {
    SEXP ans = PROTECT(allocVector(INTSXP, 3));
    INTEGER(ans)[0] = R_nchar(STRING_ELT(s, 0), Bytes, FALSE, FALSE, "capi");
    INTEGER(ans)[1] = R_nchar(STRING_ELT(s, 0), Chars, FALSE, FALSE, "capi");
    INTEGER(ans)[2] = R_nchar(STRING_ELT(s, 0), Width, FALSE, FALSE, "capi");
    UNPROTECT(1);
    return ans;
}
