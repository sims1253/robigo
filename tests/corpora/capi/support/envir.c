// Environments and symbols: define/find/remove, the special environments,
// active bindings.
// R: main/envir.c
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Rdynload.h>

// Exported by libR but not declared in any public header.
R_xlen_t Rf_envlength(SEXP);

// install/installChar/installTrChar, defineVar/findVar round trip.
SEXP capi_envir(SEXP val) {
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    SEXP sym = install("answer");
    defineVar(sym, val, env);
    SEXP got = PROTECT(findVar(sym, env));
    // N.B. the two take their arguments in opposite orders.
    Rboolean exists = R_existsVarInFrame(env, sym);
    R_removeVarFromFrame(sym, env);
    int unbound_after = findVar(sym, env) == R_UnboundValue;
    SEXP ans = PROTECT(allocVector(VECSXP, 6));
    SET_VECTOR_ELT(ans, 0, got);
    SET_VECTOR_ELT(ans, 1, ScalarLogical(exists));
    SET_VECTOR_ELT(ans, 2, ScalarLogical(unbound_after));
    SET_VECTOR_ELT(ans, 3, ScalarString(PRINTNAME(installChar(mkChar("via_installChar")))));
    // installChar is a macro for Rf_installTrChar; call Rf_installChar itself.
    SET_VECTOR_ELT(ans, 4, ScalarString(PRINTNAME(Rf_installChar(mkChar("via_Rf_installChar")))));
    SET_VECTOR_ELT(ans, 5, ScalarString(PRINTNAME(installTrChar(mkCharCE("caf\xc3\xa9", CE_UTF8)))));
    UNPROTECT(3);
    return ans;
}

// findVarInFrame vs findVar through enclosures; getVar/getVarEx.
SEXP capi_find(SEXP env, SEXP sym) {
    SEXP in_frame = PROTECT(findVarInFrame(env, sym));
    SEXP through = PROTECT(findVar(sym, env));
    SEXP gv = PROTECT(R_getVar(sym, env, TRUE));
    SEXP gvx = PROTECT(R_getVarEx(sym, env, TRUE, R_NilValue));
    SEXP ans = PROTECT(allocVector(VECSXP, 3));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(in_frame == R_UnboundValue));
    SET_VECTOR_ELT(ans, 1, through == R_UnboundValue ? mkString("<unbound>") : through);
    SET_VECTOR_ELT(ans, 2, ScalarLogical(through == gv && gv == gvx));
    UNPROTECT(5);
    return ans;
}

// findFun looks through functions only.
SEXP capi_findfun(SEXP env) {
    return findFun(install("sin"), env);
}

// The environment constants and their relations.
SEXP capi_envirs(void) {
    SEXP ans = PROTECT(allocVector(VECSXP, 9));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(R_ParentEnv(R_GlobalEnv) == R_BaseEnv));
    SET_VECTOR_ELT(ans, 1, ScalarLogical(R_BaseEnv == R_EmptyEnv ? 1 : 0));
    SET_VECTOR_ELT(ans, 2, ScalarLogical(R_ParentEnv(R_BaseEnv) == R_EmptyEnv));
    SET_VECTOR_ELT(ans, 3, ScalarInteger(Rf_envlength(R_BaseEnv) > 1000));
    SET_VECTOR_ELT(ans, 4, ScalarString(PRINTNAME(R_DotsSymbol)));
    SET_VECTOR_ELT(ans, 5, ScalarLogical(R_MissingArg == R_MissingArg));
    SET_VECTOR_ELT(ans, 6, R_BaseNamespace);
    SET_VECTOR_ELT(ans, 7, ScalarLogical(R_IsNamespaceEnv(R_BaseNamespace)));
    SET_VECTOR_ELT(ans, 8, R_lsInternal(R_BaseNamespace, TRUE));
    UNPROTECT(1);
    return ans;
}

// Active and locked bindings.
SEXP capi_active(SEXP fun) {
    SEXP env = PROTECT(R_NewEnv(R_GlobalEnv, FALSE, 0));
    SEXP sym = install("active_one");
    R_MakeActiveBinding(sym, fun, env);
    SEXP is_active = PROTECT(ScalarLogical(R_BindingIsActive(sym, env)));
    SEXP the_fun = PROTECT(R_ActiveBindingFunction(sym, env));
    R_LockBinding(sym, env);
    SEXP locked = PROTECT(ScalarLogical(R_BindingIsLocked(sym, env)));
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, is_active);
    SET_VECTOR_ELT(ans, 1, the_fun);
    SET_VECTOR_ELT(ans, 2, locked);
    SET_VECTOR_ELT(ans, 3, env);
    UNPROTECT(5);
    return ans;
}

// ... in a function frame: R_DotsSymbol, R_MissingArg for empty slots.
SEXP capi_dots(SEXP env) {
    SEXP dots = findVarInFrame(env, R_DotsSymbol);
    int n = 0, missing = 0;
    if (dots != R_UnboundValue && dots != R_MissingArg
        && (TYPEOF(dots) == LISTSXP || TYPEOF(dots) == DOTSXP)) {
        for (SEXP p = dots; p != R_NilValue; p = CDR(p)) {
            n++;
            if (CAR(p) == R_MissingArg) missing++;
        }
    }
    SEXP ans = PROTECT(allocVector(INTSXP, 2));
    INTEGER(ans)[0] = n;
    INTEGER(ans)[1] = missing;
    UNPROTECT(1);
    return ans;
}

// R_FindNamespace: the namespace of an installed package.
SEXP capi_namespace(void) {
    SEXP ns = PROTECT(R_FindNamespace(mkString("stats")));
    SEXP ans = PROTECT(allocVector(VECSXP, 2));
    SET_VECTOR_ELT(ans, 0, ScalarLogical(R_IsNamespaceEnv(ns)));
    SET_VECTOR_ELT(ans, 1, R_lsInternal(ns, TRUE));
    UNPROTECT(2);
    return ans;
}

// mkNamed builds a named vector in one go; findVarInFrame3 is the
// checking-free lookup; R_lsInternal3 lists with control over "all".
// R: main/util.c? Rf_mkNamed, main/envir.c:findVarInFrame3
SEXP capi_envir_more(SEXP env) {
    // mkNamed counts names until the empty-string sentinel.
    const char *names[4] = {"alpha", "beta", "gamma", ""};
    SEXP named = PROTECT(mkNamed(INTSXP, names));
    for (int i = 0; i < 3; i++) INTEGER(named)[i] = (i + 1) * 10;  // mkNamed leaves data uninitialized
    SEXP sym = install("known");
    defineVar(sym, ScalarInteger(7), env);
    SEXP v1 = PROTECT(findVarInFrame3(env, sym, TRUE));
    SEXP ls = PROTECT(R_lsInternal3(env, TRUE, TRUE));
    SEXP matchtab = PROTECT(allocVector(STRSXP, 3));
    SET_STRING_ELT(matchtab, 0, mkChar("a"));
    SET_STRING_ELT(matchtab, 1, mkChar("b"));
    SET_STRING_ELT(matchtab, 2, mkChar("c"));
    SEXP matched = PROTECT(match(matchtab, mkString("c"), 0));
    SEXP ans = PROTECT(allocVector(VECSXP, 4));
    SET_VECTOR_ELT(ans, 0, named);
    SET_VECTOR_ELT(ans, 1, v1);
    SET_VECTOR_ELT(ans, 2, ls);
    SET_VECTOR_ELT(ans, 3, matched);
    UNPROTECT(6);
    return ans;
}

// The punctuation symbols calls are built from, the namespace registry,
// symbol lookup and the interrupt flags.
SEXP capi_symbols(void) {
    DL_FUNC f = R_FindSymbol("capi_envir", "", NULL);
    extern SEXP R_NamespaceRegistry, R_SrcrefSymbol;
    extern int R_interrupts_pending, R_interrupts_suspended;
    extern Rboolean R_isForkedChild;  // a data flag, not a function
    SEXP ans = PROTECT(allocVector(VECSXP, 12));
    SET_VECTOR_ELT(ans, 0, ScalarString(PRINTNAME(R_BraceSymbol)));      // {
    SET_VECTOR_ELT(ans, 1, ScalarString(PRINTNAME(R_BracketSymbol)));    // [
    SET_VECTOR_ELT(ans, 2, ScalarString(PRINTNAME(R_Bracket2Symbol)));   // [[
    SET_VECTOR_ELT(ans, 3, ScalarString(PRINTNAME(R_DoubleColonSymbol)));   // ::
    SET_VECTOR_ELT(ans, 4, ScalarString(PRINTNAME(R_TripleColonSymbol)));  // :::
    SET_VECTOR_ELT(ans, 5, ScalarString(PRINTNAME(R_SrcrefSymbol)));
    SET_VECTOR_ELT(ans, 6, ScalarLogical(isEnvironment(R_NamespaceRegistry)));
    SET_VECTOR_ELT(ans, 7, ScalarLogical(f != NULL));
    SET_VECTOR_ELT(ans, 8, ScalarInteger(R_interrupts_pending));
    SET_VECTOR_ELT(ans, 9, ScalarInteger(R_interrupts_suspended));
    SET_VECTOR_ELT(ans, 10, ScalarLogical(R_isForkedChild));
    SET_VECTOR_ELT(ans, 11, ScalarInteger(length(R_lsInternal3(R_NamespaceRegistry, FALSE, TRUE)) > 0));
    UNPROTECT(1);
    return ans;
}
