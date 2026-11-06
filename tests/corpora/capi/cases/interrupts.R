dyn.load("interrupts.so")

# A user interrupt raised from C while R has interrupts suspended stays
# pending (errors.c: R_CheckUserInterrupt, onintrEx) and is processed by
# the next check after suspendInterrupts() returns. C sees the flag that
# suspendInterrupts() sets.
print(.Call("capi_interrupts_suspended"))
print(suspendInterrupts(.Call("capi_interrupts_suspended")))
print(.Call("capi_interrupts_suspended"))

r <- tryCatch(.Call("capi_interrupt"), interrupt = function(c) class(c))
print(r)

out <- "start"
r <- tryCatch(
  interrupt = function(c) c("caught later", out),
  {
    suspendInterrupts({
      tryCatch(interrupt = function(c) stop("interrupt while suspended"), .Call("capi_interrupt"))
      out <- "suspended block finished"
    })
    .Call("capi_interrupt")
    "not reached"
  }
)
print(r)
print(.Internal(interruptsSuspended()))
