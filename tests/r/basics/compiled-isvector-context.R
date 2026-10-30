# Namespace bytecode inlines is.vector before forcing X.
print(tryCatch(lapply(FUN = identity), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
# Warm each function so GNU R compiles the simple-internal wrapper call.
f <- function(x) is.vector(x)
f(1)
print(tryCatch(f(), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
g <- function() is.vector({ print(sys.calls()); 1 })
g()
g()
h <- function() is.vector(mode = { cat("mode\n"); "any" },
                         x = { cat("x\n"); 1 })
h()
h()
w <- function() is.vector({ warning("argument warning"); 1 })
suppressWarnings(w())
withCallingHandlers(w(), warning = function(e) {
    print(conditionCall(e))
    invokeRestart("muffleWarning")
})
# The runtime guard must still respect a replacement closure.
local({
    is.vector <- function(x, mode = "any") { print(sys.call()); "local" }
    f <- function(x) is.vector(x)
    print(f(1)); print(f(1))
})
