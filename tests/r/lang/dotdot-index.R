# `..n` names are the ones dstruct.c:isDDName accepts (what strtol reads
# in full after the dots), so `..0` and `..-1` read `...` and fail in
# envir.c:ddfind instead of being ordinary variables.
f <- function(...) ..0
r <- tryCatch(f(1), error = function(e) e); print(r)
r <- tryCatch(f(), error = function(e) e); print(r)
r <- tryCatch(..0, error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(eval(quote(..0)), error = function(e) e); print(r)
`..-1` <- 5
g <- function(...) `..-1`
r <- tryCatch(g(1), error = function(e) e); print(r)
h <- function(...) `..+2`
print(h(1, 7))
k <- function(...) `.. 1`
print(k(9))
`..1e` <- 3; print(`..1e`)
`..` <- 4; print(`..`)
m <- function(...) ..1
r <- tryCatch(m(), error = function(e) e); print(r)
n <- function() ..1
r <- tryCatch(n(), error = function(e) e); print(r)
p <- function(...) ...elt(0)
r <- tryCatch(p(1), error = function(e) e); print(r)
# t0 of reg-tests-1d
t0 <- function(...) ..0
for (cl in list(quote(t0()), quote(t0(1)), quote(t0(1, 2))))
    print(tryCatch(eval(cl), error = conditionMessage))
# missing() reads the first element of ... for a non-positive index
f <- function(...) missing(..0); print(f(1))
k <- function(x, ...) missing(..0); print(k())
g <- function(...) missing("..-1"); print(g(1)); print(g())
h <- function(...) { ..0 <- 5; missing(..0) }; print(h(1))
m <- function(x) missing(..0); print(tryCatch(m(1), error = conditionMessage))
# compiled code looks ..n up before it checks for a missing argument
f <- function(...) (1:3)[..0]
for (i in 1:3) print(tryCatch(f(), error = conditionMessage))
f <- function(...) (1:3)[..2]
for (i in 1:3) print(tryCatch(f(1), error = conditionMessage))
f <- function(...) (1:3)[..1]
for (i in 1:2) print(f(, 2))
