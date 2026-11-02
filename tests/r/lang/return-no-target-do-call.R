# context.c:findcontext — a return with no target context is a
# catchable error at the jump site; do.call evaluates `return(1)` in a
# fresh frame, so the error must be raisable and catchable, and
# eval(quote(return(x)), envir) is consumed by do_eval's CTXT_RETURN
# context (its value is eval's value).
print(tryCatch(do.call("return", list(1)), error = function(e) paste("E:", conditionMessage(e))))
print(eval(quote(return(1)), new.env(parent = globalenv())))
f <- function() tryCatch(do.call("return", list(1)), error = function(e) "caught")
print(f())
