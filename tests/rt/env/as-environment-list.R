# as.environment(<list>): envir.c's VECSXP case, list2env(x, NULL, emptyenv())
# evaluated in the caller's environment (so a list2env found there is the one
# called, as GNU R's eval(lang4(install("list2env"), ...), rho) does).
e <- as.environment(list(a = 1, b = "x"))
class(e)
sort(ls(e))
get("a", envir = e)
identical(parent.env(e), emptyenv())
length(ls(as.environment(list())))
# a classed list without a method falls through to the list case
f <- as.environment(structure(list(z = TRUE), class = "foo"))
get("z", envir = f)
# list2env's own errors
tryCatch(as.environment(list(1, 2)), error = function(e) conditionMessage(e))
tryCatch(as.environment(list(a = 1, 2)), error = function(e) conditionMessage(e))
# list2env is looked up from the caller
list2env <- function(x, envir, parent) "global list2env"
as.environment(list(a = 1))
rm(list2env)
g <- function() {
    list2env <- function(x, envir, parent) "local list2env"
    as.environment(list(a = 1))
}
g()
class(as.environment(list(a = 1)))
