# R: eval.c:handle_eval_depth_overflow, errors.c:R_InitConditions.
g <- function(n) g(n + 1)
f <- function(n) if (n == 0) 0 else 1 + f(n - 1)

# the classed condition
print(getOption("expressions"))
print(tryCatch(g(1), error = function(e) class(e)))
print(tryCatch(g(1), stackOverflowError = function(e) "so"))
e <- tryCatch(g(1), expressionStackOverflowError = function(e) e)
print(names(e))
print(conditionCall(e))
print(conditionMessage(e))
print(inherits(e, "simpleError"))
print(class(attr(try(g(1), silent = TRUE), "condition")))
cat(geterrmessage())

# options(expressions=) sets the limit
options(expressions = 100L)
print(tryCatch(f(1000), error = function(e) conditionMessage(e)))
print(f(10))

# a calling handler gets 500 levels of headroom, and only that
r <- tryCatch(withCallingHandlers(g(1), error = function(e) cat("calling:", f(20), "\n")),
              error = function(e) "exiting")
print(r)
r <- tryCatch(withCallingHandlers(g(1), error = function(e) cat("calling:", f(1000), "\n")),
              error = function(e) "exiting")
print(r)

# the jump to the handler puts the limit back
print(tryCatch(f(1000), error = function(e) "still limited"))

# so does a restart
options(expressions = 1000L)
print(withRestarts(withCallingHandlers(g(1), stackOverflowError = function(e) invokeRestart("r1")),
                   r1 = function() "restarted"))
print(tryCatch(f(2000), error = function(e) "still limited"))

# on.exit code during the unwind has the headroom too
options(expressions = 100L)
k <- function(n) {
    on.exit(if (n == 1L) cat("on.exit:", f(20), "\n"))
    k(n + 1L)
}
print(tryCatch(k(1L), error = function(e) "caught"))
print(tryCatch(f(1000), error = function(e) "still limited"))

# options() during the handler sets both the limit and what it goes back to
r <- tryCatch(withCallingHandlers(g(1), error = function(e) options(expressions = 200L)),
              error = function(e) "exiting")
print(r)
print(getOption("expressions"))
print(f(30))
print(tryCatch(f(1000), error = function(e) "still limited"))

# the range check
print(tryCatch(options(expressions = 10L), error = function(e) conditionMessage(e)))
print(tryCatch(options(expressions = 1000000L), error = function(e) conditionMessage(e)))

# uncaught: no call, as for every eval depth overflow
g(1)
cat("not reached\n")
