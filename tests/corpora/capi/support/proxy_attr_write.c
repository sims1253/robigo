// Writes to the attributes of objects C code keeps across calls, through
// every path the API has: setAttrib, the attribute cells (SETCAR, SETCDR,
// SET_TAG), a cell kept from ATTRIB() in an earlier call, the S4 bit. R
// sees each, after the call and from R code the routine runs.
// R: main/attrib.c, main/memory.c
#include <R.h>
#include <Rinternals.h>

static SEXP fun = NULL, env = NULL, kept_cell = NULL;

SEXP capi_aw_keep(SEXP f, SEXP e) {
    fun = f;
    env = e;
    R_PreserveObject(f);
    R_PreserveObject(e);
    return R_NilValue;
}

SEXP capi_aw_set(void) {
    Rf_setAttrib(fun, install("a"), ScalarInteger(1));
    Rf_setAttrib(env, install("a"), ScalarInteger(1));
    return R_NilValue;
}

// the first attribute cell, rewritten in place
SEXP capi_aw_setcar(void) {
    SETCAR(ATTRIB(fun), ScalarInteger(2));
    SETCAR(ATTRIB(env), ScalarInteger(2));
    return R_NilValue;
}

// a cell appended to the end of the list, then renamed
SEXP capi_aw_append(void) {
    SEXP x[2] = {fun, env};
    for (int i = 0; i < 2; i++) {
        SEXP last = ATTRIB(x[i]);
        while (CDR(last) != R_NilValue)
            last = CDR(last);
        SEXP cell = PROTECT(CONS(mkString("appended"), R_NilValue));
        SET_TAG(cell, install("b"));
        SETCDR(last, cell);
        UNPROTECT(1);
    }
    return R_NilValue;
}

SEXP capi_aw_retag(void) {
    SET_TAG(CDR(ATTRIB(env)), install("c"));
    return R_NilValue;
}

// a cell C keeps from one call and writes in a later one
SEXP capi_aw_keep_cell(void) {
    kept_cell = ATTRIB(env);
    return R_NilValue;
}

SEXP capi_aw_poke_kept(void) {
    SETCAR(kept_cell, mkString("poked later"));
    return R_NilValue;
}

SEXP capi_aw_s4(SEXP on) {
    if (asLogical(on)) {
        SET_S4_OBJECT(fun);
        SET_S4_OBJECT(env);
    } else {
        UNSET_S4_OBJECT(fun);
        UNSET_S4_OBJECT(env);
    }
    return R_NilValue;
}

// a write, then R code that reads it, in the same call
SEXP capi_aw_then_eval(SEXP reader) {
    SETCAR(ATTRIB(env), mkString("seen by R"));
    SEXP call = PROTECT(lang1(reader));
    SEXP r = Rf_eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return r;
}

SEXP capi_aw_nothing(void) {
    return R_NilValue;
}
