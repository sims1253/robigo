# A translated call's argument is byte-code promise code (a BCODESXP), and
# robigo-rt computes such an argument before the call when it can (step
# 1a). Forcing it takes one evaluation level, early value or not, and at
# the limit the force raises GNU R's error: the early path declines there
# and the ordinary force reports it.
#
# `pr` passes `id(y + 1)` (a promise of byte code), `kc` passes `id(1)` (a
# constant, no promise): with a low `expressions`, the deepest recursion
# each survives differs by the force's one level.
id <- compiler::cmpfun(function(a) a)
pr <- compiler::cmpfun(function(x, n) { y <- x; if (n == 0) id(y + 1) else pr(x, n - 1) })
kc <- compiler::cmpfun(function(x, n) { y <- x; if (n == 0) id(1) else kc(x, n - 1) })
for (i in 1:3) { pr(1, 3); kc(1, 3) }
edge <- function(fun) {
    ok <- NA
    for (n in 1:300) {
        r <- tryCatch(fun(1, n), error = function(e) conditionMessage(e))
        if (is.character(r)) return(list(last_ok = ok, first_error = n, message = r))
        ok <- n
    }
    list(last_ok = ok)
}
old <- options(expressions = 100)
p <- edge(pr)
k <- edge(kc)
options(old)
show <- function(x) cat(paste(names(x), unlist(x), sep = " = ", collapse = "; "), "\n")
show(p)
show(k)
cat("the promise's force takes one level:", k$last_ok - p$last_ok, "\n")
# and below the limit the values are the same
print(c(pr(1, 10), kc(1, 10)))
