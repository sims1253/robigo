// Experiment A's premature-finalization probe (docs/g3.md §3 item 6,
// §15.1): an external pointer held only by a C local, passed to an
// allocating API function, is not finalized during that call. GNU R's API
// functions protect their arguments while they allocate (memory.c's CONS,
// R_MakeWeakRefC, setAttrib, ...), so under gctorture the collection in
// each finds the pointer reachable (and GNU R's minor collections skip it
// once it survived one; robigo's collections are all full). Between the calls nothing allocates
// while the pointer is unprotected; it is protected while pending
// finalizers run, so only a decision taken inside the call can finalize it.
// (Not R_MakeExternalPtr's tag or prot: GNU R allocates before storing
// them, unprotected, memory.c:R_MakeExternalPtr.)
#include <R.h>
#include <Rinternals.h>

#define N 7
static int done[N];

static void fin(SEXP p) {
    int *d = (int *) R_ExternalPtrAddr(p);
    if (d) *d = 1;
    R_ClearExternalPtr(p);
}

// 1 if p was finalized (or cleared) by now
static int finalized(SEXP p, int i) {
    PROTECT(p);
    R_RunPendingFinalizers();
    int r = done[i] || R_ExternalPtrAddr(p) == NULL;
    UNPROTECT(1);
    return r;
}

SEXP capi_pp_probe(SEXP renv) {
    static const char *names[N] = {
        "R_RegisterCFinalizerEx", "Rf_defineVar (R env)", "Rf_cons", "Rf_lang2", "Rf_setAttrib", "R_MakeWeakRef", "Rf_defineVar"};
    SEXP sym = install("probe");
    SEXP val = PROTECT(ScalarInteger(1));
    SEXP env = PROTECT(R_NewEnv(R_EmptyEnv, FALSE, 0));
    int early[N];
    for (int i = 0; i < N; i++) {
        done[i] = 0;
        SEXP p = R_MakeExternalPtr(&done[i], R_NilValue, R_NilValue);
        if (i == 0) {
            R_RegisterCFinalizerEx(p, fin, FALSE);
            early[i] = finalized(p, i);
            continue;
        }
        PROTECT(p);
        R_RegisterCFinalizerEx(p, fin, FALSE);
        UNPROTECT(1);
        // p is held by this local only from here
        switch (i) {
        case 1: Rf_defineVar(sym, p, renv); break;
        case 2: Rf_cons(p, R_NilValue); break;
        case 3: Rf_lang2(sym, p); break;
        case 4: Rf_setAttrib(p, sym, val); break;
        case 5: R_MakeWeakRef(p, R_NilValue, R_NilValue, FALSE); break;
        case 6: Rf_defineVar(sym, p, env); break;
        }
        early[i] = finalized(p, i);
    }
    SEXP out = PROTECT(allocVector(LGLSXP, N));
    SEXP nms = PROTECT(allocVector(STRSXP, N));
    for (int i = 0; i < N; i++) {
        LOGICAL(out)[i] = early[i];
        SET_STRING_ELT(nms, i, mkChar(names[i]));
    }
    setAttrib(out, R_NamesSymbol, nms);
    UNPROTECT(4);
    return out;
}

