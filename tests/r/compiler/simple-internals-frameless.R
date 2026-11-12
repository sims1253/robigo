# Calls of base's simple .Internal wrappers made as closure calls (top
# level, a special's unevaluated arguments). robigo answers some without
# the frame; where the frame is observable they must behave as closure
# calls.
heads <- function() {
    s <- sys.calls()
    cat(vapply(s, function(x) paste(deparse(x[[1L]]), collapse = ""), ""), "\n")
}
x <- 1:3
n <- 2L
rep.int(x, n)
rep_len(x, length(x) + 2L)
rep.int(x, length(x))
y <- rep.int(x, n)
y
typeof(x)
identical(x, 1:3)
match(2L, x)
which.max(c(3, 9, 1))
nchar("abc")
# visibility: the value of a closure call is visible
f <- function() invisible(7)
f()
rep.int(f(), 1L)
# a warning from the internal, seen by a calling handler
k <- 2.5
withCallingHandlers(
    choose(5, k),
    warning = function(w) {
        print(conditionCall(w))
        heads()
        invokeRestart("muffleWarning")
    }
)
# an error from the internal, seen by a calling handler and tryCatch
m <- -1L
tryCatch(
    withCallingHandlers(rep.int(x, m), error = function(e) {
        print(conditionCall(e))
        heads()
    }),
    error = function(e) cat("caught:", conditionMessage(e), "\n")
)
# an active binding and an unforced promise run their code
makeActiveBinding("ab", function() { cat("active\n"); 2L }, environment())
rep.int(x, ab)
g <- function(a) rep(c(0, 1, 2), rep.int(1L, a))
g({ cat("forced\n"); 3L })
# objects dispatch
z <- factor(c("a", "b"))
rep.int(z, n)
rep_len(z, length(z) + 1L)
cl <- structure(list(1, 2), class = "foo")
length.foo <- function(x) 5L
rep.int(1, length(cl))
# a masked length()
local({
    length <- function(x) 4L
    print(rep.int(1, length(x)))
})
# a heads() frame inside the argument of a special
h <- function(X, Y) rep(Y, rep.int({ heads(); length(X) }, length(Y)))
h(1:2, 3:4)
# the internal's own call reports errors
tryCatch(rep_len(x, m), error = function(e) print(conditionCall(e)))
