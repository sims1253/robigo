// S4 objects, closures and promises.
// R: main/objects.c:R_do_slot, main/eval.c:R_mkClosure
#include <R.h>
#include <Rinternals.h>

// Slots: R_do_slot, R_do_slot_assign, R_has_slot.
SEXP capi_slots(SEXP obj, SEXP name, SEXP value) {
    int has = R_has_slot(obj, name);
    SEXP before = PROTECT(R_do_slot(obj, name));
    R_do_slot_assign(obj, name, value);
    SEXP after = PROTECT(R_do_slot(obj, name));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(has));
    SET_VECTOR_ELT(ans, 1, before);
    SET_VECTOR_ELT(ans, 2, after);
    UNPROTECT(3);
    return ans;
}

// R_do_MAKE_CLASS / R_do_new_object: build an S4 object from C.
SEXP capi_new_object(void) {
    SEXP cls = PROTECT(R_do_MAKE_CLASS("Point"));
    SEXP obj = PROTECT(R_do_new_object(cls));
    R_do_slot_assign(obj, install("x"), ScalarReal(1.5));
    R_do_slot_assign(obj, install("y"), ScalarReal(2.5));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(isS4(obj)));
    SET_VECTOR_ELT(ans, 1, obj);
    UNPROTECT(3);
    return ans;
}

// R_mkClosure builds a function; R_Closure* read it back.
SEXP capi_closure(void) {
    SEXP formals = PROTECT(list1(R_NilValue));
    SET_TAG(formals, install("x"));
    SEXP body = PROTECT(lang3(install("*"), install("x"), install("x")));
    SEXP f = PROTECT(R_mkClosure(formals, body, R_BaseEnv));
    SEXP v = PROTECT(eval(lang2(f, ScalarInteger(9)), R_GlobalEnv));
    SEXP ans = PROTECT(allocVector(VECSXP, 5));
    SET_VECTOR_ELT(ans, 0, f);
    SET_VECTOR_ELT(ans, 1, R_ClosureFormals(f));
    SET_VECTOR_ELT(ans, 2, R_ClosureBody(f));
    SET_VECTOR_ELT(ans, 3, ScalarLogical(R_ClosureEnv(f) == R_BaseEnv));
    SET_VECTOR_ELT(ans, 4, v);
    UNPROTECT(5);
    return ans;
}

// Promises: a delayed binding gives a promise; PRCODE/PRENV read it,
// SET_PRVALUE forces it by hand, SET_PRCODE/SET_PRENV rewrite it.
SEXP capi_promise_parts(SEXP env, SEXP name) {
    SEXP p = findVarInFrame(env, name);
    SEXP code = PROTECT(PRCODE(p));
    SEXP penv = PRENV(p);
    SET_PRVALUE(p, eval(code, penv));
    SET_PRCODE(p, code);  // rewrite with the same parts
    SET_PRENV(p, penv);
    SEXP val = PROTECT(PRVALUE(p));
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, code);
    SET_VECTOR_ELT(ans, 1, ScalarLogical(PRENV(p) == R_EmptyEnv));  // forced
    SET_VECTOR_ELT(ans, 2, val);
    SET_VECTOR_ELT(ans, 3, R_PromiseExpr(p));  // the bytecode-aware expr
    UNPROTECT(3);
    return ans;
}

// Body expressions of closures and promises; asS4 flips the S4 bit.
SEXP capi_exprs(SEXP f) {
    SEXP be = PROTECT(R_ClosureExpr(f));
    SEXP s4 = PROTECT(asS4(allocVector(REALSXP, 1), TRUE, 3));
    SEXP cls = PROTECT(R_getClassDef("Point"));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, be);
    SET_VECTOR_ELT(ans, 1, ScalarLogical(isS4(s4)));
    SET_VECTOR_ELT(ans, 2, ScalarLogical(cls != R_NilValue));
    UNPROTECT(4);
    return ans;
}
