# A byte-code promise of a plain symbol: a compiled closure passes a
# variable on, and the callee forces it. robigo-rt reads such a promise
# without a VM activation (eval.rs's run_single_read); what GNU R's bcEval
# does for it must hold: the value, its visibility, the errors, an active
# binding, a promise the read forces, an argument not read yet, and the
# evaluation depth.
compiler::enableJIT(3)
id <- function(a) a
pass <- function(x) id(x)
for (i in 1:3) pass(i)

# a value, and an unboxed scalar a compiled loop leaves
print(pass(42L))
scal <- function() { s <- 0; for (k in 1:5) s <- s + k; id(s) }
print(scal())
print(withVisible(pass(1))$visible)

# a missing argument
miss <- function(x) id(x)
for (i in 1:3) tryCatch(miss(1), error = function(e) NULL)
r <- tryCatch(miss(), error = function(e) conditionMessage(e))
print(r)

# an active binding, read through the promise
local({
    n <- 0
    makeActiveBinding("ab", function() { n <<- n + 1; n * 10 }, environment())
    pab <- function() id(ab)
    for (i in 1:3) pab()
    print(pab())
    print(n)
})

# a promise whose own expression errors, then is forced again
outer <- function(x) id(x)
r <- tryCatch(outer(stop("boom")), error = function(e) conditionMessage(e))
print(r)

# an argument not read yet: forced in place, once
cnt <- 0
side <- function() { cnt <<- cnt + 1; cnt }
twice <- function(x) { a <- id(x); b <- id(x); c(a, b) }
for (i in 1:3) twice(side())
print(twice(side()))
print(cnt)

# the evaluation depth: a deep chain of single reads reaches the limit as
# GNU R's does (under a lower limit: the same error path at a tenth of the
# depth, which keeps the case short under gctorture)
deep <- function(n, x) if (n == 0) id(x) else deep(n - 1, x)
for (i in 1:3) deep(5, 1)
print(deep(100, 7))
old <- options(expressions = 500)
r <- tryCatch(deep(1000, 1), error = function(e) "too deep")
options(old)
print(r)
