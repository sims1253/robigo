# Base's installed bytecode inlines is.vector before forcing X.
print(tryCatch(lapply(FUN = identity), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
print(tryCatch(sapply(FUN = identity), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
f <- function(x) lapply(x, identity)
print(f({ print(sys.calls()); 1 }))
print(f({ print(sys.calls()); 1 }))
w <- function(x) lapply(x, identity)
withCallingHandlers(w({ warning("argument warning"); 1 }), warning = function(e) {
    print(conditionCall(e))
    invokeRestart("muffleWarning")
})
print(tryCatch(lapply(stop("argument error"), identity), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
# User wrappers and replacement closures keep their frames.
local({
    is.vector <- function(x, mode = "any") { print(sys.call()); "local" }
    f <- function(x) is.vector(x)
    print(f(1)); print(f(1))
})
old <- options(warn = 2)
print(tryCatch(lapply({ warning("converted warning"); 1 }, identity),
               error = function(e) {
                   print(conditionCall(e))
                   conditionMessage(e)
               }))
options(old)
print(lapply(1, identity))
# BASEGUARD must retain a changed base wrapper as well.
local({
    original <- base::is.vector
    unlockBinding("is.vector", baseenv())
    on.exit({
        assign("is.vector", original, baseenv())
        lockBinding("is.vector", baseenv())
    })
    assign("is.vector", function(x, mode = "any") {
        print(sys.call())
        TRUE
    }, baseenv())
    print(lapply(1, identity))
})
stopifnot(identical(lapply(1, identity), list(1)))
cat("restored\n")
