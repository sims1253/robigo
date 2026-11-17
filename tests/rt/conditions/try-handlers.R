# errors.c:do_addTryHandlers: the calling function's record takes the
# internal restart "tryRestart": an error the handlers do not catch is
# reported, then the function returns R_RestartToken (eval.c's
# R_execClosure lands the restart's jump as any other).
n <- 0
f <- function() {
    n <<- n + 1
    .Internal(.addTryHandlers())
    cat("body, n =", n, "\n")
    if (n < 3) stop("again")
    n
}
print(f())
print(n)
v <- f()
print(typeof(v))
print(nchar(as.character(v)))
g <- function() { .Internal(.addTryHandlers()); print(length(computeRestarts())); 1 }
print(g())
m <- 0
h2 <- function() {
    m <<- m + 1
    on.exit(cat("h2 exits", m, "\n"))
    .Internal(.addTryHandlers())
    if (m < 2) invokeRestart("tryRestart")
    "done"
}
print(h2())
print(h2())
print(tryCatch(.Internal(.addTryHandlers()), error = conditionMessage))
w <- function() { .Internal(.addTryHandlers()); warning("ww"); "after a warning" }
print(w())
c1 <- function() { .Internal(.addTryHandlers()); tryCatch(stop("caught"), error = function(e) conditionMessage(e)) }
print(c1())
