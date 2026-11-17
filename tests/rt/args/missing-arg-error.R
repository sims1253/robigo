# errors.c:R_MissingArgError: a missing argument is a classed
# missingArgError, its subclass the site's: eval's evalError, byte code's
# getvarError, get()'s and findFun's getMissingError.
show <- function(e) {
    cat(class(e), "|", conditionMessage(e), "|", deparse(conditionCall(e)), "\n")
    print(names(unclass(e)))
}
f <- function(x) x
show(tryCatch(f(), error = identity))
k <- function(x) { for (i in 1) y <- x; y }
show(tryCatch(k(), error = identity))
show(tryCatch(get("x", envir = (function(x) environment())()), error = identity))
m <- function(g) g()
show(tryCatch(m(), error = identity))
d <- function(...) ..1
show(tryCatch(d(, 2), error = identity))
print(tryCatch(f(), missingArgError = function(e) "missingArgError caught"))
print(tryCatch(k(), getvarError = function(e) "getvarError caught"))
r <- withCallingHandlers(tryCatch(f(), error = function(e) "exiting"),
                         missingArgError = function(e) cat("calling handler first\n"))
print(r)
print(class(attr(try(f(), silent = TRUE), "condition")))
cat(geterrmessage())
f()
