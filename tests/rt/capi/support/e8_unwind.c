// E8 across a C frame (docs/g3.md §15.3), for tests/rt/capi/e8-unwind.R:
// the three unwind cases of crates/robigo-rtapi/tests/capi.rs (its C in
// tests/support/capi_test.c), recorded on GNU R 4.6.2 so that GNU R gives
// the expectations. Rprintf logs the order of events.
// R: main/context.c:R_UnwindProtect, R_ExecWithCleanup, R_ToplevelExec
#include <R.h>
#include <Rinternals.h>

// an error raised from C
SEXP e8_error(SEXP msg)
{
    error("%s", CHAR(STRING_ELT(msg, 0)));
    return R_NilValue;
}

// ---- 1: on.exit code that catches its own error -------------------------

// .Call("e8_error", "inner") evaluated from here: R code between
static void eval_stop_inner(void *data)
{
    (void) data;
    SEXP call = PROTECT(lang3(install(".Call"), mkString("e8_error"), mkString("inner")));
    eval(call, R_GlobalEnv);
    UNPROTECT(1);
}

// on.exit code: catches the error it raises itself (R_ToplevelExec)
SEXP e8_catch_inner(void)
{
    Rboolean ok = R_ToplevelExec(eval_stop_inner, NULL);
    Rprintf("on.exit: %s\n", ok ? "inner returned" : "inner caught");
    return R_NilValue;
}

// f() evaluated from C: this routine's frame is between the error and the
// catcher
SEXP e8_eval_call(SEXP f)
{
    SEXP call = PROTECT(lang1(f));
    SEXP v = eval(call, R_GlobalEnv);
    UNPROTECT(1);
    Rprintf("e8_eval_call returned\n");
    return v;
}

// ---- 2: an R_UnwindProtect cleanup that catches resumes the original
// continuation ------------------------------------------------------------

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
    Rprintf("cleanup: %s\n", jump ? "jump" : "no jump");
    Rprintf("cleanup: %s\n", ok ? "returned" : "caught its own");
}

SEXP e8_unwind_catching_cleanup(void)
{
    SEXP cont = PROTECT(R_MakeUnwindCont());
    R_UnwindProtect(body_original, NULL, cleanup_catches, NULL, cont);
    UNPROTECT(1);
    Rprintf("after R_UnwindProtect\n");
    return R_NilValue;
}

// ---- 3: a C context's cend runs before the jump --------------------------

typedef struct {
    volatile int *magic;
} cend_data;

static SEXP ewc_fails(void *data)
{
    (void) data;
    SEXP call = PROTECT(lang3(install(".Call"), mkString("e8_error"), mkString("from the body")));
    eval(call, R_GlobalEnv);
    UNPROTECT(1);
    return R_NilValue;
}

static void ewc_cleanup(void *data)
{
    cend_data *d = data;
    // the routine's frame, and so the record's, is still there
    Rprintf("cleanup: %s\n", *d->magic == 0x5eed ? "before the jump" : "after the jump");
}

SEXP e8_exec_with_cleanup_fails(void)
{
    volatile int magic = 0x5eed;
    cend_data d = { &magic };
    R_ExecWithCleanup(ewc_fails, NULL, ewc_cleanup, &d);
    magic = 0;
    Rprintf("R_ExecWithCleanup returned\n");
    return R_NilValue;
}
