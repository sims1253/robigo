# errors.c's C-level tryCatch (R_tryCatch, R_tryCatchError,
# R_withCallingErrorHandler and .Internal(C_tryCatchHelper)), its C in
# support/trycatch.c.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/trycatch.so"))

# the body returns: its value
print(.Call("capi_tc_error", quote(1 + 1)))
# an R error, then a C error, caught by R_tryCatchError's handler
print(.Call("capi_tc_error", quote(stop("an R error"))))
print(.Call("capi_tc_error_c"))
# classes: a warning caught as a condition, an error not caught here
print(.Call("capi_tc", quote(warning("a warning")), c("warning", "error"), FALSE))
print(tryCatch(.Call("capi_tc", quote(stop("passes")), "warning", FALSE),
               error = function(e) paste("outer:", conditionMessage(e))))
# finally runs after the body and after a handler
print(.Call("capi_tc", quote(42), "error", TRUE))
print(.Call("capi_tc", quote(stop("then finally")), "error", TRUE))
# a classed condition
print(.Call("capi_tc", quote(signalCondition(structure(class = c("custom", "condition"),
                                                         list(message = "custom one", call = NULL)))),
            "custom", FALSE))
# defaults: no conditions and no handler
print(.Call("capi_tc_defaults", quote("plain value")))
print(tryCatch(.Call("capi_tc_defaults", quote(stop("no handler"))), error = conditionMessage))
# no body
print(tryCatch(.Call("capi_tc_nobody"), error = conditionMessage))
# interrupts stay as they were
print(.Internal(interruptsSuspended()))
# R_withCallingErrorHandler: the calling handler, then the outer handler
print(.Call("capi_wceh", quote(3)))
print(tryCatch(.Call("capi_wceh", quote(stop("calling first"))),
               error = function(e) paste("outer:", conditionMessage(e))))
print(tryCatch(.Call("capi_wceh_nobody"), error = conditionMessage))
