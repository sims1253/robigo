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
    for (int i = 0; i < k; i++)
        PROTECT(R_NilValue);
    UNPROTECT(k);
    return ScalarInteger(k);
}
