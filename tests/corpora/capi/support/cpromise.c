// A promise assembled in C: allocSExp(PROMSXP) with SET_PRVALUE,
// SET_PRCODE and SET_PRENV, as rlang's r_env_bind_delayed binds `.`
// in magrittr's pipe masks (src/rlang/env-binding.c). The evaluator
// must force it when R reads the binding, all bindings of one node
// share the forcing, a NULL code is a promise of NULL, an already-set
// PRVALUE is honored, and C keeps reading and writing the node after
// the fact — including across gc().
// R: main/memory.c:allocSExp, Rinternals.h:SET_PRVALUE/SET_PRCODE/SET_PRENV
#include <R.h>
#include <Rinternals.h>

static SEXP kept = NULL;

SEXP capi_cpromise(void) {
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, 0, 0));
    defineVar(install("value"), ScalarInteger(21), env);

    // { value <- value + 1; value }: forcing bumps the counter, so a
    // second forcing of the same promise would be observable
    SEXP expr = PROTECT(lang3(install("{"),
                              lang3(install("<-"),
                                    install("value"),
                                    lang3(install("+"), install("value"), ScalarInteger(1))),
                              install("value")));
    SEXP p = PROTECT(allocSExp(PROMSXP));
    SET_PRVALUE(p, R_UnboundValue);
    SET_PRCODE(p, expr);
    SET_PRENV(p, env);
    defineVar(install("x"), p, env);
    defineVar(install("y"), p, env);
    kept = p;

    // NULL code: a promise of NULL
    SEXP pnull = PROTECT(allocSExp(PROMSXP));
    SET_PRVALUE(pnull, R_UnboundValue);
    SET_PRCODE(pnull, R_NilValue);
    SET_PRENV(pnull, env);
    defineVar(install("z"), pnull, env);

    // forced with NULL code keeps its value
    SEXP pf = PROTECT(allocSExp(PROMSXP));
    SET_PRVALUE(pf, ScalarInteger(7));
    SET_PRCODE(pf, R_NilValue);
    SET_PRENV(pf, R_NilValue);
    defineVar(install("w"), pf, env);

    UNPROTECT(5);
    return env;
}

// C writes the original node after the promise was bound and forced
SEXP capi_cpromise_poke(void) {
    if (kept == NULL) {
        return R_NilValue;
    }
    SET_PRVALUE(kept, ScalarInteger(99));
    return R_NilValue;
}

// C writes the node through the cons setter the SET_PR* macro expands to
SEXP capi_cpromise_setcar(void) {
    if (kept == NULL) {
        return R_NilValue;
    }
    SETCAR(kept, ScalarInteger(77));
    return R_NilValue;
}

// Whether C still sees the promise as unforced (PRVALUE == R_UnboundValue)
SEXP capi_cpromise_is_unbound(void) {
    if (kept == NULL) {
        return ScalarLogical(NA_LOGICAL);
    }
    return ScalarLogical(PRVALUE(kept) == R_UnboundValue);
}
