# eval.c:handle_eval_depth_overflow, errors.c:R_InitConditions; options.c:
# options(expressions =) sets R_Expressions and R_Expressions_keep. The
# limits stay below the depth at which robigo-rt's C stack runs out.
g <- function(n) g(n + 1)
f <- function(n) if (n == 0) 0 else 1 + f(n - 1)
options(expressions = 500L)
print(getOption("expressions"))
e <- tryCatch(g(1), error = identity)
print(class(e))
print(names(unclass(e)))
print(conditionCall(e))
print(conditionMessage(e))
print(tryCatch(g(1), stackOverflowError = function(e) "so"))
print(tryCatch(g(1), expressionStackOverflowError = function(e) "expression"))
print(tryCatch(f(1000), error = function(e) conditionMessage(e)))
print(f(100))
# a calling handler gets 500 levels of headroom, and only that
r <- tryCatch(withCallingHandlers(g(1), error = function(e) cat("calling:", f(20), "\n")),
              error = function(e) "exiting")
print(r)
r <- tryCatch(withCallingHandlers(g(1), error = function(e) cat("calling:", f(1200), "\n")),
              error = function(e) "exiting")
print(r)
# the jump to the handler puts the limit back
print(tryCatch(f(1000), error = function(e) "still limited"))
# options() during the handler sets both the limit and what it goes back to
r <- tryCatch(withCallingHandlers(g(1), error = function(e) options(expressions = 200L)),
              error = function(e) "exiting")
print(r)
print(getOption("expressions"))
print(f(30))
print(tryCatch(f(300), error = function(e) "still limited"))
print(class(attr(try(g(1), silent = TRUE), "condition")))
cat(geterrmessage())
# the range check
print(tryCatch(options(expressions = 10L), error = function(e) conditionMessage(e)))
print(tryCatch(options(expressions = 1000000L), error = function(e) conditionMessage(e)))
print(getOption("expressions"))
# uncaught: no call
g(1)
cat("not reached\n")
