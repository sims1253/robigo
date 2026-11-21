# DispatchOrEval with no method (eval.c): robigo skips the setup when the
# method search, memoized, finds none without running code. Each way of
# making a method afterwards is seen at the next dispatch, from byte code
# (loops compile the functions) and from the AST; a lookup that would run
# code (an active binding, a promise) takes the whole way and runs it.
x <- structure(list(a = 1, b = 2), class = "k")
d <- function() x$a
s <- function() x[2]
l <- function() length(x)
show <- function() print(list(d(), s(), l(), eval(quote(x$a)), eval(quote(x[2]))))
for (i in 1:3) show()
# a method assigned in the global environment
`$.k` <- function(x, name) paste("dollar", name)
`[.k` <- function(x, i) paste("bracket", i)
length.k <- function(x) 99L
for (i in 1:2) show()
rm(`$.k`, `[.k`, length.k)
for (i in 1:2) show()
# a method in a frame on the path (the caller's)
g <- function(def) {
    if (def) `$.k` <- function(x, name) "local"
    x$a
}
for (i in 1:3) print(g(FALSE))
print(g(TRUE))
print(g(FALSE))
# registerS3method into base's S3 methods table, then removed from it
for (i in 1:2) print(d())
registerS3method("$", "k", function(x, name) "registered", envir = baseenv())
for (i in 1:2) print(d())
tbl <- get(".__S3MethodsTable__.", envir = baseenv())
rm(list = "$.k", envir = tbl)
for (i in 1:2) print(d())
# a write to the table itself
assign("[.k", function(x, i) "table", envir = tbl)
for (i in 1:2) print(s())
rm(list = "[.k", envir = tbl)
for (i in 1:2) print(s())
# attach and detach
attach(list(`$.k` = function(x, name) "attached"), name = "meths")
for (i in 1:2) print(d())
detach("meths")
for (i in 1:2) print(d())
# an active binding on the path: GNU R's lookup calls it each time
n <- 0
makeActiveBinding("$.k", function() { n <<- n + 1; NULL }, globalenv())
for (i in 1:3) print(d())
print(n)
rm(`$.k`)
# a promise on the path: the lookup forces it
delayedAssign("[.k", { cat("forced\n"); function(x, i) "lazy" })
for (i in 1:2) print(s())
rm(`[.k`)
for (i in 1:2) print(s())
# `...` in the call: promiseArgs reads the caller's (here none)
f <- function() x[...]
for (i in 1:2) print(tryCatch(f(), error = function(e) conditionMessage(e)))
h <- function(...) x[...]
for (i in 1:2) print(h(1))
