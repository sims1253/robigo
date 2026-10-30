# User closures inline simple `.Internal` wrappers once compiled, with
# a BASEGUARD on the base binding (R: compiler/R/cmp.R:cmpSimpleInternal,
# eval.c:DO_BASEGUARD). Forcing order, error and warning calls match.
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
# The runtime guard still respects a replacement closure.
local({
    is.vector <- function(x, mode = "any") { print(sys.call()); "local" }
    f <- function(x) is.vector(x)
    print(f(1)); print(f(1))
})
# Partial matching aligns to formals before substituting.
p <- function(v) is.vector(mo = "any", x = v)
print(p(1)); print(p(1))
# Error calls deparse the substituted actuals.
r <- function(v) rep.int(v, -1)
print(tryCatch({ r(1); r(1) }, error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
n <- function(v) nchar(v, type = 2)
print(tryCatch({ n("a"); n("a") }, error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
m <- function(v) is.vector(v, 2)
print(tryCatch({ m(1); m(1) }, error = function(e) {
    print(conditionCall(e))
    conditionMessage(e)
}))
# Unforced promises evaluate in formal order.
e <- function() {
    p <- { cat("p\n"); 1 }
    mo <- { cat("mo\n"); "any" }
    is.vector(p, mo)
}
e()
e()
# A global rebinding after compilation is honored.
q <- function(x) is.vector(x)
print(q(2)); print(q(2))
assign("is.vector", function(x, mode = "any") "rebound", globalenv())
print(q(3))
rm("is.vector", envir = globalenv())
print(q(4))
