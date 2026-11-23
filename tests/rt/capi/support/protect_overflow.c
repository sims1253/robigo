/* PROTECT past R_PPStackSize (memory.c:R_signal_protect_error): the
 * protectStackOverflowError condition, with a NULL call; its calling
 * handlers run in the red zone, and the size is restored after. */
#include <R.h>
#include <Rinternals.h>

/* PROTECT(R_NilValue) n times, then UNPROTECT them: n past the stack's
 * room overflows it */
SEXP rt_protect_n(SEXP n)
{
    int k = asInteger(n);
    if (k == NA_INTEGER || k < 0)
        error("rt_protect_n: 'n' must be a count (a non-negative integer)");
    for (int i = 0; i < k; i++)
        PROTECT(R_NilValue);
    UNPROTECT(k);
    return ScalarInteger(k);
}

/* R_PPStackSize: GNU R's LibExtern global; robigo's is the isolate's */
#ifdef ROBIGO_CAPI_H_
#define PP_SIZE (robigo_cg->R_PPStackSize)
#else
extern int R_PPStackSize;
#define PP_SIZE R_PPStackSize
#endif

SEXP rt_pp_size(void)
{
    return ScalarInteger(PP_SIZE);
}

typedef struct {
    SEXP expr, rho;
} eval_data;

static SEXP eval_body(void *data)
{
    eval_data *d = data;
    return eval(d->expr, d->rho);
}

static void report_size(void *data)
{
    (void) data;
    Rprintf("the cleanup sees R_PPStackSize %d\n", PP_SIZE);
}

/* expr evaluated with a cleanup that reports R_PPStackSize: an exit out
 * of an overflow's calling handler passes the overflow's context first,
 * which restores the size */
SEXP rt_size_in_cleanup(SEXP expr, SEXP rho)
{
    eval_data d = { expr, rho };
    return R_ExecWithCleanup(eval_body, &d, report_size, NULL);
}
