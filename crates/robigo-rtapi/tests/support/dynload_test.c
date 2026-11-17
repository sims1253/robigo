/* A package's shared object for robigo-rtapi's dyn.load tests
 * (tests/dynload.rs), built as packages are: against GNU R 4.6.2's headers
 * with robigo's Rinternals.h in front. Built twice: DL_NAME names the
 * object (R_init_<DL_NAME> and R_unload_<DL_NAME> are found by it), and
 * DL_FORCE makes its init force native symbols, as tools' does. */
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>
#include <stddef.h>

#ifndef DL_NAME
#error "DL_NAME names the object"
#endif
#define CAT2(a, b) a##b
#define CAT(a, b) CAT2(a, b)
#define STR2(a) #a
#define STR(a) STR2(a)

/* GNU R's registration layout (R_ext/Rdynload.h, LP64), which
 * robigo-rtapi's rdynload.rs asserts too */
_Static_assert(sizeof(R_CMethodDef) == 32, "R_CMethodDef");
_Static_assert(offsetof(R_CMethodDef, name) == 0, "R_CMethodDef.name");
_Static_assert(offsetof(R_CMethodDef, fun) == 8, "R_CMethodDef.fun");
_Static_assert(offsetof(R_CMethodDef, numArgs) == 16, "R_CMethodDef.numArgs");
_Static_assert(offsetof(R_CMethodDef, types) == 24, "R_CMethodDef.types");
_Static_assert(sizeof(R_CallMethodDef) == 24, "R_CallMethodDef");
_Static_assert(offsetof(R_CallMethodDef, name) == 0, "R_CallMethodDef.name");
_Static_assert(offsetof(R_CallMethodDef, fun) == 8, "R_CallMethodDef.fun");
_Static_assert(offsetof(R_CallMethodDef, numArgs) == 16, "R_CallMethodDef.numArgs");
_Static_assert(sizeof(R_ExternalMethodDef) == sizeof(R_CallMethodDef), "R_ExternalMethodDef");
_Static_assert(sizeof(R_FortranMethodDef) == sizeof(R_CMethodDef), "R_FortranMethodDef");

/* .Call: x + 1L */
static SEXP dl_add1(SEXP x)
{
    return ScalarInteger(asInteger(x) + 1);
}

/* .Call, any number of arguments: how many */
static SEXP dl_nargs(SEXP a, SEXP b, SEXP c)
{
    (void) a; (void) b; (void) c;
    return ScalarInteger(3);
}

/* the object's record, as its init got it (per thread: each test's
 * isolate loads the object on its own thread) */
static __thread DllInfo *self;

/* .Call: the lookup settings changed after calls by name, as an init sets
 * them: later calls by name do not find the routines */
static SEXP dl_force_now(void)
{
    R_useDynamicSymbols(self, FALSE);
    R_forceSymbols(self, TRUE);
    return R_NilValue;
}

/* .External: the pairlist it gets, as a list with its tags as names */
static SEXP dl_ext(SEXP args)
{
    return PairToVectorList(args);
}

/* .External2: list(call, op, args, env) */
static SEXP dl_ext2(SEXP call, SEXP op, SEXP args, SEXP env)
{
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, call);
    SET_VECTOR_ELT(ans, 1, op);
    SET_VECTOR_ELT(ans, 2, PairToVectorList(args));
    SET_VECTOR_ELT(ans, 3, env);
    UNPROTECT(1);
    return ans;
}

/* .External2 that raises an error: the caller's frame unwinds through the
 * builtin's record */
static SEXP dl_ext2_error(SEXP call, SEXP op, SEXP args, SEXP env)
{
    (void) op; (void) args; (void) env;
    errorcall(call, "dl_ext2_error was called");
    return R_NilValue;
}

/* .C (registered only: robigo-rt has no .C) */
static void dl_c(int *x)
{
    (void) x;
}

static R_NativePrimitiveArgType dl_c_types[] = {INTSXP};

static const R_CMethodDef CEntries[] = {
    {"dl_c", (DL_FUNC) &dl_c, 1, dl_c_types},
    {NULL, NULL, 0, NULL}
};

static const R_CallMethodDef CallEntries[] = {
    {"dl_add1", (DL_FUNC) &dl_add1, 1},
    {"dl_nargs", (DL_FUNC) &dl_nargs, -1},
    {"dl_force_now", (DL_FUNC) &dl_force_now, 0},
    {NULL, NULL, 0}
};

static const R_ExternalMethodDef ExtEntries[] = {
    {"dl_ext", (DL_FUNC) &dl_ext, -1},
    {"dl_ext2", (DL_FUNC) &dl_ext2, 2},
    {"dl_ext2_error", (DL_FUNC) &dl_ext2_error, -1},
    {NULL, NULL, 0}
};

/* not registered: found by dlsym while the object allows dynamic lookup */
SEXP dl_unregistered(void)
{
    return mkString("unregistered");
}

void CAT(R_init_, DL_NAME)(DllInfo *dll)
{
    self = dll;
    R_registerRoutines(dll, CEntries, CallEntries, NULL, ExtEntries);
#ifdef DL_FORCE
    R_useDynamicSymbols(dll, FALSE);
    R_forceSymbols(dll, TRUE);
#endif
    Rprintf("R_init_%s\n", STR(DL_NAME));
}

void CAT(R_unload_, DL_NAME)(DllInfo *dll)
{
    (void) dll;
    Rprintf("R_unload_%s\n", STR(DL_NAME));
}
