# .Internal(Recall(...)): the calling closure called again with Recall's
# arguments, from the environment it was called from (eval.c do_recall).
f <- function(n) if (n <= 1) 1 else n * Recall(n - 1)
f(5)
# the function is the record's, not found by name
g <- f
rm(f)
g(4)
# named and default arguments: Recall's arguments are matched afresh
h <- function(x, y = 2) if (x > 0) Recall(x - 1, y) else y
h(3, 7)
h(x = 2)
h(y = 9, x = 1)
# the call each level sees is the original call
k <- function(n) {
  cat(deparse(sys.call()), "\n")
  if (n > 0) Recall(n - 1) else n
}
k(2)
# missingness is passed through Recall's promises
m <- function(a, b) if (missing(b)) "no b" else Recall(a)
m(1, 2)
# anonymous and list-held functions
(function(n) if (n) Recall(n - 1) else "anon")(2)
lst <- list(fn = function(n) if (n) Recall(n - 1) else "list")
lst$fn(2)
do.call(function(n) if (n) Recall(n - 1) else "do.call", list(3))
sapply(1:3, function(n) if (n > 1) n * Recall(n - 1) else 1)
# visibility is the recursive call's
v <- function(n) if (n) Recall(n - 1) else invisible("hidden")
v(2)
print(v(2))
# arguments are evaluated once, in the caller's frame
cnt <- 0
w <- function(n) if (n > 0) Recall({ cnt <<- cnt + 1; n - 1 }) else cnt
w(3)
# a compiled caller (a loop makes the JIT compile it)
fl <- function(n) {
  s <- 0
  for (i in seq_len(2)) s <- s + i
  if (n <= 1) s else n + Recall(n - 1)
}
fl(4)
fl(5)
# an S3 method: the record's function is the method
cnt.foo <- function(x, n) if (n > 0) Recall(x, n - 1) else class(x)
cnt <- function(x, n) UseMethod("cnt")
cnt(structure(1, class = "foo"), 3)
# errors: outside a closure
r <- tryCatch(Recall(1), error = function(e) e)
r
conditionCall(r)
q <- function() .Internal(Recall(1))
r <- tryCatch(q(), error = function(e) e)
r
conditionCall(r)
# the caller is eval()'s record, whose function is not a closure
e1 <- function() eval(quote(Recall(1)))
r <- tryCatch(e1(), error = function(e) e)
r
conditionCall(r)
r <- tryCatch(evalq(Recall(), new.env()), error = function(e) e)
r
