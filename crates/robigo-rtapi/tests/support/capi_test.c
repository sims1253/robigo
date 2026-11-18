/* Package code for robigo-rtapi's tests (tests/capi.rs), built against GNU R
 * 4.6.2's headers with robigo's Rinternals.h in front, as packages are, and
 * called through .Call. Each routine says which test uses it. State lives
 * in thread-locals: each test runs its own isolate on its own thread. */
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Altrep.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* a log the tests read back (rt_log_take) */
static __thread char logbuf[4096];

static void logmsg(const char *m)
{
    size_t len = strlen(logbuf);
    if (len + 2 > sizeof logbuf)
        return; /* full: the message and its ';' do not fit */
    strncat(logbuf, m, sizeof logbuf - len - 2);
    strcat(logbuf, ";");
}

SEXP rt_log_take(void)
{
    SEXP s = mkString(logbuf);
    logbuf[0] = '\0';
    return s;
}

/* ---- the no-op .Call (its cost per crossing: the ledger) ------------- */

SEXP rt_noop3(SEXP a, SEXP b, SEXP c)
{
    (void) a; (void) b; (void) c;
    return R_NilValue;
}

/* ---- E8 1: an on.exit that catches its own error during an unwind ---- */

/* an error raised from C (R's stop() is the condition system's) */
SEXP rt_error(SEXP msg)
{
    error("%s", CHAR(STRING_ELT(msg, 0)));
    return R_NilValue;
}

/* .Call("rt_error", "inner") evaluated from here: R code between */
static void eval_stop_inner(void *data)
{
    (void) data;
    SEXP call = PROTECT(lang3(install(".Call"), mkString("rt_error"), mkString("inner")));
    eval(call, R_GlobalEnv);
    UNPROTECT(1);
}

/* on.exit code: catches the error it raises itself (R_ToplevelExec) */
SEXP rt_catch_inner(void)
{
    Rboolean ok = R_ToplevelExec(eval_stop_inner, NULL);
    logmsg(ok ? "inner returned" : "inner caught");
    return R_NilValue;
}

/* f() evaluated from C: the frame of this routine is between the error and
 * the catcher */
SEXP rt_eval_call(SEXP f)
{
    SEXP call = PROTECT(lang1(f));
    SEXP v = eval(call, R_GlobalEnv);
    UNPROTECT(1);
    logmsg("rt_eval_call returned");
    return v;
}

/* ---- E8 2: an R_UnwindProtect cleanup that catches resumes the
 * original continuation ------------------------------------------------ */

static SEXP body_original(void *data)
{
    (void) data;
    error("original");
    return R_NilValue;
}

static void raise_cleanups(void *data)
{
    (void) data;
    error("the cleanup's own");
}

static void cleanup_catches(void *data, Rboolean jump)
{
    (void) data;
    Rboolean ok = R_ToplevelExec(raise_cleanups, NULL);
    logmsg(jump ? "cleanup: jump" : "cleanup: no jump");
    logmsg(ok ? "cleanup: returned" : "cleanup: caught its own");
}

SEXP rt_unwind_catching_cleanup(void)
{
    SEXP cont = PROTECT(R_MakeUnwindCont());
    R_UnwindProtect(body_original, NULL, cleanup_catches, NULL, cont);
    UNPROTECT(1);
    logmsg("after R_UnwindProtect");
    return R_NilValue;
}

/* ---- E8 3: a C context's cend runs before the jump -------------------- */

typedef struct {
    volatile int *magic;
} cend_data;

static SEXP ewc_fails(void *data)
{
    (void) data;
    SEXP call = PROTECT(lang3(install(".Call"), mkString("rt_error"), mkString("from the body")));
    eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return R_NilValue;
}

static void ewc_cleanup(void *data)
{
    cend_data *d = data;
    /* the routine's frame (and so the record's) is still there */
    logmsg(*d->magic == 0x5eed ? "cleanup: before the jump" : "cleanup: after the jump");
}

SEXP rt_exec_with_cleanup_fails(void)
{
    volatile int magic = 0x5eed;
    cend_data d = { &magic };
    R_ExecWithCleanup(ewc_fails, NULL, ewc_cleanup, &d);
    magic = 0;
    logmsg("R_ExecWithCleanup returned");
    return R_NilValue;
}

/* ---- nested native re-entry ---------------------------------------- */

/* f(n - 1) through eval while this frame holds a fresh vector only in C
 * locals and the protect stack; a collection in each level */
SEXP rt_nested(SEXP f, SEXP n)
{
    int k = asInteger(n);
    SEXP mine = PROTECT(allocVector(INTSXP, 64));
    for (int i = 0; i < 64; i++)
        INTEGER(mine)[i] = k * 1000 + i;
    SEXP inner = R_NilValue;
    if (k > 0) {
        SEXP call = PROTECT(lang2(f, ScalarInteger(k - 1)));
        inner = eval(call, R_GlobalEnv);
        UNPROTECT(1);
    }
    PROTECT(inner);
    R_gc();
    int sum = 0;
    for (int i = 0; i < 64; i++)
        sum += INTEGER(mine)[i] - k * 1000;
    SEXP ans = ScalarInteger(sum + (k > 0 ? asInteger(inner) : 0));
    UNPROTECT(2);
    return ans;
}

/* ---- cpp11's unwind protection: the cleanup leaves by longjmp ---------- */

static SEXP cpp11_body(void *data)
{
    SEXP call = PROTECT(lang3(install(".Call"), mkString("rt_error"), mkString((const char *) data)));
    eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return R_NilValue;
}

static void cpp11_cleanup(void *jmpbuf, Rboolean jump)
{
    if (jump == TRUE)
        longjmp(*(jmp_buf *) jmpbuf, 1);
}

/* cpp11's unwind_protect (cpp11/protect.hpp) without C++: the cleanup
 * longjmps out of R_UnwindProtect back here, past its frame, and the code
 * here (cpp11's catch of unwind_exception at END_CPP11) resumes the unwind
 * with R_ContinueUnwind */
/* cpp11's continuation: one per session (cpp11 keeps it in a static and
 * preserves it); the tests keep it in an R variable */
SEXP rt_make_token(void)
{
    return R_MakeUnwindCont();
}

SEXP rt_cpp11(SEXP token, SEXP msg)
{
    int top_before = robigo_cg->R_PPStackTop;
    jmp_buf jmpbuf;
    if (setjmp(jmpbuf)) {
        /* (cpp11 throws, and its .Call wrapper catches) */
        logmsg(robigo_cg->R_PPStackTop == top_before ? "longjmp landed, stack as before" : "longjmp landed, stack moved");
        R_ContinueUnwind(token);
    }
    SEXP res = R_UnwindProtect(cpp11_body, (void *) CHAR(STRING_ELT(msg, 0)), cpp11_cleanup, &jmpbuf, token);
    return res;
}

/* a collection after the routine above: every segment the scan reads must
 * be live */
SEXP rt_collect(void)
{
    R_gc();
    return ScalarInteger(1);
}

/* ---- an R_alloc block, and L8: allocation parity ----------------------- */

/* an R_alloc block whose address C keeps only in a static across a full
 * collection: still where C's pointer points (no C stack word pins it, so
 * the moving debug collector must know the block is C's) */
static char *volatile r_alloc_saved;

static void __attribute__((noinline)) r_alloc_fill(void)
{
    r_alloc_saved = R_alloc(16, 1);
    snprintf(r_alloc_saved, 16, "kept");
}

SEXP rt_r_alloc_survives(void)
{
    r_alloc_fill();
    R_gc();
    SEXP top = (SEXP) vmaxget();
    return ScalarLogical((char *) RAW(top) == r_alloc_saved && strcmp(r_alloc_saved, "kept") == 0);
}

/* L8's probe, experiment A's method: an external pointer held only by this
 * C local, with a collection due (its registration allocated last), then
 * one call; TRUE if that call collected (the pointer is then finalized).
 * GNU R allocates on none of the paths `what` names but "new name", "cons"
 * and "tryEval constant" (protectedEval's R_PreserveObject of the value). */
static __thread int probe_done[1024];
static __thread int probe_k;

/* (each pointer's own flag: an earlier probe's pointer, finalized later,
 * must not count for this one) */
static void probe_fin(SEXP p)
{
    int *d = R_ExternalPtrAddr(p);
    if (d)
        *d = 1;
}

SEXP rt_collects_in(SEXP what, SEXP x)
{
    const char *w = CHAR(STRING_ELT(what, 0));
    int *done = &probe_done[probe_k++ % 1024];
    *done = 0;
    SEXP p = R_MakeExternalPtr(done, R_NilValue, R_NilValue);
    R_RegisterCFinalizer(p, probe_fin);
    /* p is held by this local only, and a collection is due */
    if (!strcmp(w, "install known")) install("names");
    else if (!strcmp(w, "new name")) install(CHAR(STRING_ELT(x, 0)));
    else if (!strcmp(w, "mkChar cached")) mkChar("names");
    else if (!strcmp(w, "ScalarLogical")) ScalarLogical(1);
    else if (!strcmp(w, "setAttrib existing")) setAttrib(x, R_NamesSymbol, getAttrib(x, R_NamesSymbol));
    else if (!strcmp(w, "getAttrib row.names")) getAttrib(x, R_RowNamesSymbol);
    else if (!strcmp(w, "getAttrib absent")) getAttrib(x, R_DimSymbol);
    else if (!strcmp(w, "defineVar bound")) defineVar(install("bound"), x, R_GlobalEnv);
    else if (!strcmp(w, "PROTECT")) { PROTECT(x); UNPROTECT(1); }
    else if (!strcmp(w, "cons")) Rf_cons(x, R_NilValue);
    else if (!strcmp(w, "tryEval constant")) R_tryEval(x, R_GlobalEnv, NULL);
    else error("rt_collects_in: no case %s", w);
    R_RunPendingFinalizers();
    return ScalarLogical(*done);
}

/* ---- experiment A's probes (capi corpus pin_premature, pin_extbuf) ---- */

#define N 7
static __thread int done[N];

static void fin(SEXP p)
{
    int *d = (int *) R_ExternalPtrAddr(p);
    if (d) *d = 1;
    R_ClearExternalPtr(p);
}

static int finalized(SEXP p, int i)
{
    PROTECT(p);
    R_RunPendingFinalizers();
    int r = done[i] || R_ExternalPtrAddr(p) == NULL;
    UNPROTECT(1);
    return r;
}

SEXP capi_pp_probe(SEXP renv)
{
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

static __thread void *freed;

static void fin_free(SEXP p)
{
    void *b = R_ExternalPtrAddr(p);
    if (b) {
        freed = b;
        free(b);
    }
    R_ClearExternalPtr(p);
}

SEXP capi_eb_probe(void)
{
    char *buf = malloc(64);
    if (buf == NULL)
        error("capi_eb_probe: out of memory");
    strcpy(buf, "in use");
    freed = NULL;
    SEXP p = R_MakeExternalPtr(buf, R_NilValue, R_NilValue);
    R_RegisterCFinalizerEx(p, fin_free, FALSE);
    R_gc();
    R_RunPendingFinalizers();
    int during = freed == buf;
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

/* ---- misuse: objects of another isolate, exports with no isolate ----- */

/* (process-wide on purpose: the misuse is across isolates) */
static SEXP stash;

SEXP rt_stash(void)
{
    stash = PROTECT(ScalarInteger(7));
    R_PreserveObject(stash);
    UNPROTECT(1);
    return R_NilValue;
}

SEXP rt_use_stash(void)
{
    return Rf_cons(stash, R_NilValue);
}

/* called directly, not through .Call: no isolate runs C code here */
SEXP rt_outside(void)
{
    return ScalarInteger(1);
}

/* errors from accessors and the protect stack */
SEXP rt_integer_of(SEXP x)
{
    return ScalarInteger(INTEGER(x)[0]);
}

SEXP rt_leave_protected(void)
{
    PROTECT(ScalarInteger(1));
    return R_NilValue;
}

SEXP rt_over_unprotect(void)
{
    UNPROTECT(1);
    return R_NilValue;
}

SEXP rt_warn(void)
{
    warning("a warning from C: %d", 3);
    return ScalarInteger(1);
}

/* R_UnwindProtect with R_NilValue for its continuation (not the NULL that
 * asks for one) */
static SEXP nil_cont_fun(void *data)
{
    (void) data;
    logmsg("fun");
    return R_NilValue;
}

static void nil_cont_clean(void *data, Rboolean jump)
{
    (void) data;
    (void) jump;
    logmsg("cleanup");
}

SEXP rt_unwind_nil(void)
{
    return R_UnwindProtect(nil_cont_fun, NULL, nil_cont_clean, NULL, R_NilValue);
}

/* n x's, for error() and Rprintf() of long texts (an R_alloc block: the
 * tests' sessions run on several threads at once) */
static const char *xs(SEXP n)
{
    int k = asInteger(n);
    if (k == NA_INTEGER || k < 0)
        k = 0;
    char *text = R_alloc((size_t) k + 1, 1);
    memset(text, 'x', (size_t) k);
    text[k] = '\0';
    return text;
}

SEXP rt_error_n(SEXP n)
{
    error("%s", xs(n));
    return R_NilValue;
}

SEXP rt_print_n(SEXP n)
{
    Rprintf("%s", xs(n));
    return R_NilValue;
}

/* A package ALTREP integer of length 1 with no other method (a class per
   call: each test's isolate has its own registry). */
static R_xlen_t one_length(SEXP x)
{
    return 1;
}

SEXP rt_altrep_one(void)
{
    R_altrep_class_t c = R_make_altinteger_class("rt_one", "rtcapi", NULL);
    R_set_altrep_Length_method(c, one_length);
    return R_new_altrep(c, R_NilValue, R_NilValue);
}

/* coerceVector to a type code from R (one too large for any SEXPTYPE) */
SEXP rt_coerce_to(SEXP x, SEXP type)
{
    return coerceVector(x, (SEXPTYPE) asReal(type));
}

/* The package classes' Length method over data1; it counts its calls in
   data2, an integer scalar (rt_altrep_length_calls: per object, so tests
   running in parallel do not share the count). */
/* A new instance of class `c` over `v`, its count at 0. */
static SEXP new_counted(R_altrep_class_t c, SEXP v)
{
    SEXP n = PROTECT(ScalarInteger(0));
    SEXP r = R_new_altrep(c, v, n);
    UNPROTECT(1);
    return r;
}

static R_xlen_t data1_length(SEXP x)
{
    SEXP n = R_altrep_data2(x);
    INTEGER(n)[0]++;
    return XLENGTH(R_altrep_data1(x));
}
static SEXP str_elt(SEXP x, R_xlen_t i) { return STRING_ELT(R_altrep_data1(x), i); }

/* A package ALTSTRING over the character vector `v`, its Length and Elt
   methods reading it (B's probe class). */
SEXP rt_altrep_str(SEXP v)
{
    R_altrep_class_t c = R_make_altstring_class("rt_str", "rtcapi", NULL);
    R_set_altrep_Length_method(c, data1_length);
    R_set_altstring_Elt_method(c, str_elt);
    return new_counted(c, v);
}

/* Package ALTINTEGER, ALTREAL and ALTLIST classes over the vector `v`, the
   same way (the attribute setters' cases). */

/* How many times `x`'s Length method ran since the last call. */
SEXP rt_altrep_length_calls(SEXP x)
{
    SEXP n = R_altrep_data2(x);
    int c = INTEGER(n)[0];
    INTEGER(n)[0] = 0;
    return ScalarInteger(c);
}
static int int_elt(SEXP x, R_xlen_t i) { return INTEGER_ELT(R_altrep_data1(x), i); }
static double real_elt(SEXP x, R_xlen_t i) { return REAL_ELT(R_altrep_data1(x), i); }
static SEXP list_elt(SEXP x, R_xlen_t i) { return VECTOR_ELT(R_altrep_data1(x), i); }

SEXP rt_altrep_int(SEXP v)
{
    R_altrep_class_t c = R_make_altinteger_class("rt_int", "rtcapi", NULL);
    R_set_altrep_Length_method(c, data1_length);
    R_set_altinteger_Elt_method(c, int_elt);
    return new_counted(c, v);
}

SEXP rt_altrep_real(SEXP v)
{
    R_altrep_class_t c = R_make_altreal_class("rt_real", "rtcapi", NULL);
    R_set_altrep_Length_method(c, data1_length);
    R_set_altreal_Elt_method(c, real_elt);
    return new_counted(c, v);
}

SEXP rt_altrep_list(SEXP v)
{
    R_altrep_class_t c = R_make_altlist_class("rt_list", "rtcapi", NULL);
    R_set_altrep_Length_method(c, data1_length);
    R_set_altlist_Elt_method(c, list_elt);
    return new_counted(c, v);
}
