// CHARSXP encodings: the marks mkCharCE sets, cache identity per encoding,
// translation (and its refusal of "bytes"), reEnc, ucstoutf8, R_nchar and
// the locale data libR exports.
// R: main/envir.c:mkCharLenCE, main/sysutils.c:getCharCE, translateChar*,
// reEnc, ucstoutf8, main/character.c:R_nchar, main/platform.c:R_check_locale
#include <R.h>
#include <Rinternals.h>
#include <R_ext/GraphicsEngine.h>  // GraphicsDevice.h: mbcslocale, Rf_ucstoutf8
#include <string.h>

// LibExtern in GNU R's Defn.h, so a package can link them
extern Rboolean utf8locale, known_to_be_latin1;
extern int R_MB_CUR_MAX;
// exported, declared only in Defn.h
extern const char *Rf_translateCharFP(SEXP);

static SEXP bytes_of(const char *s) {
    size_t n = strlen(s);
    SEXP r = allocVector(RAWSXP, n);
    memcpy(RAW(r), s, n);
    return r;
}

// getCharCE, the ASCII mark and charIsUTF8/charIsLatin1 of "\xc3\xa9" made
// in each encoding, and of ASCII text made in each encoding
SEXP capi_enc_marks(void) {
    const cetype_t enc[] = {CE_NATIVE, CE_UTF8, CE_LATIN1, CE_BYTES};
    SEXP ans = PROTECT(allocMatrix(INTSXP, 4, 8));
    for (int i = 0; i < 4; i++) {
        SEXP x = mkCharCE("\xc3\xa9", enc[i]);
        SEXP a = mkCharCE("abc", enc[i]);
        int *col = INTEGER(ans) + i;
        col[0] = getCharCE(x);
        col[4] = charIsASCII(x);
        col[8] = charIsUTF8(x);
        col[12] = charIsLatin1(x);
        col[16] = getCharCE(a);
        col[20] = charIsASCII(a);
        col[24] = charIsUTF8(a);
        col[28] = charIsLatin1(a);
    }
    UNPROTECT(1);
    return ans;
}

// the same bytes in the same encoding are one CHARSXP; in different
// encodings they are different ones, except for ASCII text
SEXP capi_enc_identity(void) {
    const cetype_t enc[] = {CE_NATIVE, CE_UTF8, CE_LATIN1, CE_BYTES};
    SEXP ans = PROTECT(allocVector(LGLSXP, 7));
    SEXP x[4];
    for (int i = 0; i < 4; i++) x[i] = PROTECT(mkCharCE("caf\xc3\xa9", enc[i]));
    int same = 1, distinct = 1;
    for (int i = 0; i < 4; i++) {
        same &= mkCharCE("caf\xc3\xa9", enc[i]) == x[i];
        same &= mkCharLenCE("caf\xc3\xa9!", 5, enc[i]) == x[i];
        for (int j = i + 1; j < 4; j++) distinct &= x[i] != x[j];
    }
    LOGICAL(ans)[0] = same;
    LOGICAL(ans)[1] = distinct;
    LOGICAL(ans)[2] = mkChar("caf\xc3\xa9") == x[0];
    LOGICAL(ans)[3] = mkCharCE("plain", CE_UTF8) == mkChar("plain");
    LOGICAL(ans)[4] = mkCharCE("plain", CE_BYTES) == mkChar("plain");
    LOGICAL(ans)[5] = mkCharCE("plain", CE_SYMBOL) == mkChar("plain");
    LOGICAL(ans)[6] = mkCharCE("plain", CE_ANY) == mkChar("plain");
    UNPROTECT(5);
    return ans;
}

// a CHARSXP made in encoding `enc` (from raw bytes)
SEXP capi_enc_mk(SEXP raw, SEXP enc) {
    return ScalarString(mkCharLenCE((const char *) RAW(raw), LENGTH(raw), (cetype_t) asInteger(enc)));
}

// CE_SYMBOL and CE_ANY non-ASCII text: only what is cached as native
SEXP capi_enc_symbol_any(SEXP enc) {
    cetype_t e = (cetype_t) asInteger(enc);
    SEXP n = PROTECT(mkCharCE("\xce\xb1-cached", CE_NATIVE));
    SEXP ans = ScalarLogical(mkCharCE("\xce\xb1-cached", e) == n);
    UNPROTECT(1);
    return ans;
}
SEXP capi_enc_symbol_new(SEXP enc) {
    return ScalarString(mkCharCE("\xce\xb1-not-cached", (cetype_t) asInteger(enc)));
}

// translateChar (0), translateCharUTF8 (1) and translateCharFP (2) of the
// first element, as raw bytes
SEXP capi_enc_translate(SEXP s, SEXP how) {
    SEXP x = STRING_ELT(s, 0);
    const char *r;
    switch (asInteger(how)) {
    case 0: r = translateChar(x); break;
    case 1: r = translateCharUTF8(x); break;
    default: r = Rf_translateCharFP(x); break;
    }
    return bytes_of(r);
}

// whether translation returns CHAR(x) itself
SEXP capi_enc_translate_same(SEXP s) {
    SEXP x = STRING_ELT(s, 0);
    SEXP ans = allocVector(LGLSXP, 2);
    LOGICAL(ans)[0] = translateChar(x) == CHAR(x);
    LOGICAL(ans)[1] = translateCharUTF8(x) == CHAR(x);
    return ans;
}

// getCharCE of something that is not a CHARSXP
SEXP capi_enc_not_charsxp(SEXP x, SEXP how) {
    switch (asInteger(how)) {
    case 0: return ScalarInteger(getCharCE(x));
    case 1: return mkString(translateCharUTF8(x));
    default: return ScalarInteger(charIsASCII(x));
    }
}

// reEnc(raw bytes, from, to, subst), as raw bytes
SEXP capi_enc_reenc(SEXP raw, SEXP from, SEXP to, SEXP subst) {
    int n = LENGTH(raw);
    char *s = R_alloc(n + 1, 1);
    memcpy(s, RAW(raw), n);
    s[n] = 0;
    const char *r = reEnc(s, (cetype_t) asInteger(from), (cetype_t) asInteger(to), asInteger(subst));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, bytes_of(r));
    SET_VECTOR_ELT(ans, 1, ScalarLogical(r == s));
    UNPROTECT(1);
    return ans;
}

// ucstoutf8(code point): the bytes and the count it returns
SEXP capi_enc_ucs(SEXP cp) {
    char buf[16];
    memset(buf, 'x', sizeof buf);
    size_t n = Rf_ucstoutf8(buf, (unsigned int) asReal(cp));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, bytes_of(buf));
    SET_VECTOR_ELT(ans, 1, ScalarInteger((int) n));
    UNPROTECT(1);
    return ans;
}

// R_nchar(x, type, allowNA, keepNA, msg_name or NULL)
SEXP capi_enc_nchar(SEXP s, SEXP type, SEXP allow, SEXP msg) {
    SEXP x = STRING_ELT(s, 0);
    const char *m = isNull(msg) ? NULL : CHAR(STRING_ELT(msg, 0));
    return ScalarInteger(R_nchar(x, (nchar_type) asInteger(type), asLogical(allow), FALSE, m));
}

// the locale data
SEXP capi_enc_locale(void) {
    SEXP ans = PROTECT(allocVector(INTSXP, 4));
    INTEGER(ans)[0] = mbcslocale;
    INTEGER(ans)[1] = utf8locale;
    INTEGER(ans)[2] = known_to_be_latin1;
    INTEGER(ans)[3] = R_MB_CUR_MAX;
    UNPROTECT(1);
    return ans;
}

// symbols from CHARSXPs: installChar translates, as installTrChar does
SEXP capi_enc_install(SEXP s) {
    SEXP x = STRING_ELT(s, 0);
    SEXP a = installTrChar(x);
    SEXP b = installChar(x);
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(a == b));
    SET_VECTOR_ELT(ans, 1, bytes_of(CHAR(PRINTNAME(a))));
    SET_VECTOR_ELT(ans, 2, ScalarString(PRINTNAME(a)));
    UNPROTECT(1);
    return ans;
}
