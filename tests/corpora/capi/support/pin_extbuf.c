// Experiment A's external-buffer probe (docs/g3.md §15.1): an external
// pointer held only by a C local, whose finalizer frees the buffer the
// pointer names while another C local still names it. GNU R decides
// finalizers on precise reachability (memory.c:CheckFinalizers), so a full
// collection (R_gc; a minor one would not look at the pointer again once
// it survived the allocations of R_RegisterCFinalizerEx) finds it
// unreachable, and the pending finalizers free the buffer: the
// resource-safety limit of unprotected C code, which pins (stack words keep
// memory, not finalizer decisions) share. Nothing reads the buffer after it
// may be freed; the finalizer records which buffer it freed.
#include <R.h>
#include <Rinternals.h>
#include <stdlib.h>
#include <string.h>

static void *freed;

static void fin_free(SEXP p) {
    void *b = R_ExternalPtrAddr(p);
    if (b) {
        freed = b;
        free(b);
    }
    R_ClearExternalPtr(p);
}

SEXP capi_eb_probe(void) {
    char *buf = malloc(64);
    strcpy(buf, "in use");
    freed = NULL;
    SEXP p = R_MakeExternalPtr(buf, R_NilValue, R_NilValue);
    R_RegisterCFinalizerEx(p, fin_free, FALSE);
    // p is held by this local only, buf by another
    R_gc();
    R_RunPendingFinalizers();
    int during = freed == buf;
    // the pointer object itself is not freed yet: the finalizer kept it
    // for this round, and nothing collected since
    int ptr_cleared = TYPEOF(p) == EXTPTRSXP && R_ExternalPtrAddr(p) == NULL;
    SEXP out = PROTECT(allocVector(LGLSXP, 2));
    LOGICAL(out)[0] = during;
    LOGICAL(out)[1] = ptr_cleared;
    SEXP nms = PROTECT(allocVector(STRSXP, 2));
    SET_STRING_ELT(nms, 0, mkChar("freed_while_named"));
    SET_STRING_ELT(nms, 1, mkChar("pointer_cleared"));
    setAttrib(out, R_NamesSymbol, nms);
    UNPROTECT(2);
    return out;
}
