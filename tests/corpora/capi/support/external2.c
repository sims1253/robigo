// .External2: the routine gets (call, op, args, rho), and the primitive
// leaves R_Visible as the routine's last evaluation set it (names.c eval
// code 201), so a routine that evaluates an assignment returns invisibly
// (magrittr's `%<>%`). .External always returns visibly (eval code 1).
// R: main/names.c:R_FunTab, main/eval.c:eval (PRIMPRINT), main/dotcode.c:do_External
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

// Evaluate each argument (a language object) in rho; return the last value.
static SEXP ext2_eval(SEXP call, SEXP op, SEXP args, SEXP rho) {
    SEXP out = R_NilValue;
    for (args = CDR(args); args != R_NilValue; args = CDR(args))
        out = eval(CAR(args), rho);
    return out;
}

// The same through .External: no rho, so evaluate in the global env.
static SEXP ext1_eval(SEXP args) {
    SEXP out = R_NilValue;
    for (args = CDR(args); args != R_NilValue; args = CDR(args))
        out = eval(CAR(args), R_GlobalEnv);
    return out;
}

static const R_ExternalMethodDef ExtEntries[] = {
    {"ext2_eval", (DL_FUNC)&ext2_eval, -1},
    {"ext1_eval", (DL_FUNC)&ext1_eval, -1},
    {NULL, NULL, 0}};

void R_init_external2(DllInfo *dll) {
    R_registerRoutines(dll, NULL, NULL, NULL, ExtEntries);
}
