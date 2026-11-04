// Class checks from C: R_check_class_etc and R_data_class.
// R: main/objects.c:R_check_class_and_super, main/attrib.c:R_data_class
#include <R.h>
#include <Rinternals.h>

// not in the API headers (Defn.h); the methods package calls it
SEXP R_data_class(SEXP obj, Rboolean singleString);

static const char *valid[] = {"Other", "foo", "Base", "numeric", "V", ""};

// The index of class(x) in valid, or -1.
SEXP capi_check_class(SEXP x) {
    return ScalarInteger(R_check_class_etc(x, valid));
}

// R_data_class(x, FALSE) and R_data_class(x, TRUE).
SEXP capi_data_class(SEXP x) {
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, R_data_class(x, FALSE));
    SET_VECTOR_ELT(ans, 1, R_data_class(x, TRUE));
    UNPROTECT(1);
    return ans;
}
