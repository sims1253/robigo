# Installed base bytecode inlines simple `.Internal` wrappers before
# forcing their arguments (R: compiler/R/cmp.R:cmpSimpleInternal).
# Missing arguments, `...`, shadowing and lookup failures stay generic.
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
# `%in%` calls match(x, table, nomatch = 0L) from base namespace code.
print(1:3 %in% c(2, 4))
print(tryCatch(match(1:3, "a"), error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
# Extra and duplicated arguments stay generic.
g <- function(a) rep.int(a, -1, "extra")
print(tryCatch({ g(1); g(1) }, error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
h <- function(a, b) rep.int(a, b, b)
print(tryCatch({ h(1, -1); h(1, -1) }, error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
# A local wrapper keeps its frame.
local({
    is.vector <- function(x, mode = "any") { print(sys.call()); "local" }
    f <- function(x) is.vector(x)
    print(f(1)); print(f(1))
})
# A rebinding in base itself does not replace the installed internal.
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
# Nor does a rebinding done before the first call.
local({
    original <- base::match
    unlockBinding("match", baseenv())
    on.exit({
        assign("match", original, baseenv())
        lockBinding("match", baseenv())
    })
    assign("match", function(x, table, nomatch = NA_integer_, incomparables = NULL) {
        print(sys.call())
        0L
    }, baseenv())
    print(2 %in% c(1, 2))
})
# A global rebinding is invisible to installed callers too.
local({
    assign("is.vector", function(x, mode = "any") {
        print(sys.call())
        TRUE
    }, globalenv())
    on.exit(rm("is.vector", envir = globalenv()))
    print(lapply(1, identity))
})
stopifnot(identical(lapply(1, identity), list(1)))
cat("restored\n")
