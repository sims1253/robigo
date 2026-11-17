# User interrupts (errors.c:onintrEx, signalInterrupt): SIGINT raised by
# support/interrupt.c, taken at R_CheckUserInterrupt or at a poll.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/interrupt.so"))

# exiting handlers catch it: class interrupt, or condition
r <- tryCatch(.Call("capi_interrupt"), interrupt = function(c) c)
print(class(r))
print(length(unclass(r)))
print(tryCatch(.Call("capi_interrupt"), condition = function(c) "a condition handler"))
# a calling handler sees it, then the resume restart goes on
r <- withCallingHandlers({
    .Call("capi_interrupt")
    "after the interrupt"
}, interrupt = function(c) {
    cat("calling handler; first restart:", computeRestarts(c)[[1L]][[1L]], "\n")
    invokeRestart("resume")
})
print(r)
# not an error: an error handler does not see it
print(tryCatch(tryCatch(.Call("capi_interrupt"), error = function(e) "error handler"),
               interrupt = function(c) "interrupt handler"))
# on.exit code runs as it unwinds
f <- function() {
    on.exit(cat("on.exit of f\n"))
    .Call("capi_interrupt")
}
print(tryCatch(f(), interrupt = function(c) "caught after f"))
# options(interrupt) runs after the calling handlers
options(interrupt = function() cat("options(interrupt)\n"))
print(tryCatch(withCallingHandlers(.Call("capi_interrupt"),
                                   interrupt = function(c) cat("calling first\n")),
               interrupt = function(c) "exiting last"))
options(interrupt = NULL)
# suspended: pending until the suspension ends and a poll takes it
g <- function() {
    old <- .Internal(interruptsSuspended(TRUE))
    .Call("capi_interrupt")
    cat("still running while suspended\n")
    .Internal(interruptsSuspended(old))
    for (i in 1:100000) NULL
    "not reached"
}
print(tryCatch(g(), interrupt = function(c) "taken after the suspension"))
# taken at a later poll of a loop
print(tryCatch({
    .Call("capi_raise")
    for (i in 1:1000000) NULL
    "not reached"
}, interrupt = function(c) "taken at a poll"))
# uncaught: the script ends
.Call("capi_interrupt")
cat("not reached\n")
