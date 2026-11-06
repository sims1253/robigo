// A missing argument with a default is bound to a promise of the default
// in the new frame, a constant default included, compiled or not
// (eval.c:applyClosure: mkPROMISE(CAR(f), newrho)). C code that reads the
// binding without forcing it sees the promise (rlang's
// env_binding_are_lazy()).
// R: main/eval.c:applyClosure
#include <R.h>
#include <Rinternals.h>

static const char *state(SEXP v) {
    if (v == R_UnboundValue) return "unbound";
    if (TYPEOF(v) != PROMSXP) return "value";
    return PRVALUE(v) == R_UnboundValue ? "promise" : "forced";
}

// The bindings of `syms` in `env`, read without forcing them.
SEXP capi_binding_states(SEXP env, SEXP syms) {
    R_xlen_t n = XLENGTH(syms);
    SEXP out = PROTECT(allocVector(STRSXP, n));
    for (R_xlen_t i = 0; i < n; i++)
        SET_STRING_ELT(out, i, mkChar(state(findVarInFrame(env, installChar(STRING_ELT(syms, i))))));
    UNPROTECT(1);
    return out;
}
