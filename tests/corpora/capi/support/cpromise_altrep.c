// A promise assembled in C (as cpromise.c, and as dplyr's data mask binds
// its chops), forced by R code after the .Call returned, while a
// package's ALTREP class has an instance. The runtime then writes the
// forced value back to the C node with no native call on the stack: the
// write-back must not need the interpreter of a call (#533's crash in
// dplyr's arrange).
// R: main/memory.c:allocSExp, main/altrep.c:R_new_altrep
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>
#include <R_ext/Rdynload.h>

static R_altrep_class_t wrap_class;
static SEXP kept = NULL;

static R_xlen_t w_Length(SEXP x) { return XLENGTH(R_altrep_data1(x)); }
static void *w_Dataptr(SEXP x, Rboolean writeable) { return (void *) INTEGER(R_altrep_data1(x)); }
static int w_Elt(SEXP x, R_xlen_t i) { return INTEGER(R_altrep_data1(x))[i]; }

void R_init_cpromise_altrep(DllInfo *dll) {
    wrap_class = R_make_altinteger_class("wrap_int", "cpromise_altrep", dll);
    R_set_altrep_Length_method(wrap_class, w_Length);
    R_set_altvec_Dataptr_method(wrap_class, w_Dataptr);
    R_set_altinteger_Elt_method(wrap_class, w_Elt);
}

// env with x and y bound to one unforced promise of `value + 1L`, and v
// an instance of the package class
SEXP capi_cpromise_altrep(void) {
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, 0, 0));
    defineVar(install("value"), ScalarInteger(41), env);
    SEXP v = PROTECT(R_new_altrep(wrap_class, PROTECT(ScalarInteger(3)), R_NilValue));
    defineVar(install("v"), v, env);
    SEXP p = PROTECT(allocSExp(PROMSXP));
    SET_PRVALUE(p, R_UnboundValue);
    SET_PRCODE(p, lang3(install("+"), install("value"), ScalarInteger(1)));
    SET_PRENV(p, env);
    defineVar(install("x"), p, env);
    defineVar(install("y"), p, env);
    kept = p;
    R_PreserveObject(p);
    UNPROTECT(4);
    return env;
}

// what C reads in the node after R forced it
SEXP capi_cpromise_altrep_value(void) {
    return PRVALUE(kept);
}
