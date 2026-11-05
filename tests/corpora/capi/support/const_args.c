// How a constant argument reaches a closure: GNU R's AST interpreter
// promises every argument (eval.c:promiseArgs), byte-compiled call sites
// push constants as values (bcEval PUSHCONSTARG; compiler cmpCallArgs).
// Which call sites run as bytecode follows R_CheckJIT (eval.c): installed
// package code, closures the JIT compiled, and top-level loops at JIT
// level 3; eval(), C's Rf_eval and do.call() always interpret. rlang's
// dots_info / check_dots_used tell "delayed" (an unforced promise) from a
// value this way.
// R: main/eval.c:promiseArgs, R_CheckJIT, bcEval; library/compiler/R/cmp.R
#include <R.h>
#include <Rinternals.h>

static const char *state(SEXP v) {
    if (TYPEOF(v) != PROMSXP) return "value";
    return PRVALUE(v) == R_UnboundValue ? "promise" : "forced";
}

// The binding of `sym` in `env`, read without forcing it.
SEXP capi_arg_state(SEXP env, SEXP sym) {
    return mkString(state(findVarInFrame(env, sym)));
}

// Each element of `...` in `env`, read without forcing it.
SEXP capi_dots_state(SEXP env) {
    SEXP dots = findVarInFrame(env, R_DotsSymbol);
    int n = 0;
    for (SEXP p = dots; TYPEOF(p) == DOTSXP || TYPEOF(p) == LISTSXP; p = CDR(p)) n++;
    SEXP out = PROTECT(allocVector(STRSXP, n));
    int i = 0;
    for (SEXP p = dots; TYPEOF(p) == DOTSXP || TYPEOF(p) == LISTSXP; p = CDR(p))
        SET_STRING_ELT(out, i++, mkChar(state(CAR(p))));
    UNPROTECT(1);
    return out;
}

// A call built and evaluated by C: GNU R's AST interpreter, wherever the
// caller is.
SEXP capi_c_call(SEXP fun, SEXP env) {
    SEXP call = PROTECT(lang2(fun, ScalarInteger(1)));
    SEXP r = eval(call, env);
    UNPROTECT(1);
    return r;
}
