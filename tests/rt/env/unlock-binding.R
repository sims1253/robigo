# unlockBinding: the binding written again after lockBinding; its errors
show <- function(expr) tryCatch(expr, error = function(e) cat("Error in", deparse(conditionCall(e)), ":", conditionMessage(e), "\n"))
e <- new.env()
assign("x", 1, envir = e)
lockBinding("x", e)
bindingIsLocked("x", e)
show(assign("x", 2, envir = e))
unlockBinding("x", e)
bindingIsLocked("x", e)
assign("x", 3, envir = e)
get("x", envir = e)
unlockBinding("x", e)  # invisible NULL
lockEnvironment(e, bindings = TRUE)
show(assign("x", 4, envir = e))
unlockBinding("x", e)
assign("x", 5, envir = e)
get("x", envir = e)
show(assign("y", 1, envir = e))
show(unlockBinding("nosuch", e))
show(unlockBinding(quote(x), e))
show(.Internal(unlockBinding("x", e)))
show(.Internal(unlockBinding(quote(x), 1)))
show(.Internal(unlockBinding(quote(x), NULL)))
f <- function() { z <- 1; lockBinding("z", environment()); unlockBinding("z", environment()); z <- 2; z }
f()
a <- 1
lockBinding("a", globalenv())
unlockBinding("a", globalenv())
a <- 2
a
