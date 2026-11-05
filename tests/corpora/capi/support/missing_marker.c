#include <R.h>
#include <Rinternals.h>

/* R_MissingArg (the symbol "") seen from C: its value through an
   environment chain, as rlang's capture of a missing argument and its
   r_env_until() do. */
SEXP capi_missing_marker(SEXP env) {
    SEXP out = PROTECT(allocVector(LGLSXP, 4));
    LOGICAL(out)[0] = findVar(R_MissingArg, env) == R_MissingArg;
    LOGICAL(out)[1] = findVar(R_MissingArg, env) == R_UnboundValue;
    LOGICAL(out)[2] = R_existsVarInFrame(R_BaseEnv, R_MissingArg);
    LOGICAL(out)[3] = findVarInFrame(R_BaseEnv, R_MissingArg) == R_MissingArg;
    UNPROTECT(1);
    return out;
}

SEXP capi_eval_missing_marker(SEXP env) {
    return eval(R_MissingArg, env);
}
