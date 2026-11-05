#include <R.h>
#include <Rinternals.h>

extern int R_interrupts_suspended;
extern int R_interrupts_pending;

/* As rlang's r_interrupt(): raise a user interrupt from C. */
SEXP capi_interrupt(void) {
    R_interrupts_pending = 1;
    R_CheckUserInterrupt();
    return R_NilValue;
}

/* What C sees of R's suspendInterrupts(). */
SEXP capi_interrupts_suspended(void) {
    return ScalarLogical(R_interrupts_suspended);
}
