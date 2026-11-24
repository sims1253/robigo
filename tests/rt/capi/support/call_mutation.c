// Writes from package C into the call a closure gets from sys.call():
// lapply's FUN(X[[i]], ...), whose X[[i]] robigo-rt shares between
// invocations (docs/ledger/g3/2026-11-23-apply-fcall.md). GNU R builds a
// fresh call each time, so a later invocation sees none of these.
#include <R.h>
#include <Rinternals.h>

// X[[i]]'s index set to 1L
SEXP rt_call_set_index(SEXP call) {
    SEXP one = PROTECT(ScalarInteger(1));
    SETCADDR(CADR(call), one);
    UNPROTECT(1);
    return R_NilValue;
}

// X[[i]]'s X tagged `changed`
SEXP rt_call_set_tag(SEXP call) {
    SET_TAG(CDR(CADR(call)), install("changed"));
    return R_NilValue;
}

// an attribute `a` on X[[i]]
SEXP rt_call_set_attr(SEXP call) {
    SEXP one = PROTECT(ScalarInteger(1));
    setAttrib(CADR(call), install("a"), one);
    UNPROTECT(1);
    return R_NilValue;
}
