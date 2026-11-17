// User interrupts: SIGINT raised in the process, taken at
// R_CheckUserInterrupt or at R's own poll points.
// R: main/errors.c:onintrEx, signalInterrupt; main/main.c:handleInterrupt
#include <R.h>
#include <Rinternals.h>
#include <R_ext/Utils.h>
#include <signal.h>

// SIGINT, then the poll: the interrupt is taken here.
SEXP capi_interrupt(void) {
    raise(SIGINT);
    R_CheckUserInterrupt();
    Rprintf("C: after R_CheckUserInterrupt\n");
    return R_NilValue;
}

// SIGINT only: the interrupt waits for the next poll.
SEXP capi_raise(void) {
    raise(SIGINT);
    return R_NilValue;
}
