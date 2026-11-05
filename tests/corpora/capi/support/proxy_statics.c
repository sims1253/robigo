// Objects C code keeps in statics are the caller's objects: attributes the
// routine writes to them in a later call, when they are not its arguments,
// are R's attributes. GNU R has one object; robigo keeps the runtime object
// and its proxy in step when the routine returns.
// R: main/attrib.c, main/memory.c
#include <R.h>
#include <Rinternals.h>

static SEXP kept_env = NULL, kept_fun = NULL, kept_other = NULL;

SEXP capi_static_keep(SEXP env, SEXP fun, SEXP other) {
    kept_env = env;
    kept_fun = fun;
    kept_other = other;
    R_PreserveObject(env);
    R_PreserveObject(fun);
    R_PreserveObject(other);
    return R_NilValue;
}

// setAttrib on objects that are not arguments
SEXP capi_static_classify(void) {
    Rf_setAttrib(kept_env, R_ClassSymbol, mkString("kept"));
    Rf_setAttrib(kept_fun, install("note"), mkString("from C"));
    Rf_setAttrib(kept_other, install("size"), ScalarInteger(3));
    return R_NilValue;
}

// the attribute cells written in place, as installAttrib does
SEXP capi_static_poke(void) {
    for (SEXP a = ATTRIB(kept_env); a != R_NilValue; a = CDR(a))
        if (TAG(a) == R_ClassSymbol)
            SETCAR(a, mkString("poked"));
    SEXP cell = PROTECT(CONS(ScalarLogical(1), R_NilValue));
    SET_TAG(cell, install("added"));
    SETCDR(ATTRIB(kept_fun), cell);
    UNPROTECT(1);
    return R_NilValue;
}

// the whole attribute list replaced, and the S4 bit
SEXP capi_static_replace(void) {
    SEXP cell = PROTECT(CONS(mkString("only"), R_NilValue));
    SET_TAG(cell, install("tag"));
    SET_ATTRIB(kept_other, cell);
    SET_S4_OBJECT(kept_env);
    UNPROTECT(1);
    return R_NilValue;
}

SEXP capi_static_clear(void) {
    CLEAR_ATTRIB(kept_env);
    UNSET_S4_OBJECT(kept_env);
    Rf_setAttrib(kept_fun, install("note"), R_NilValue);
    return R_NilValue;
}

SEXP capi_static_nothing(SEXP x) {
    return R_NilValue;
}
