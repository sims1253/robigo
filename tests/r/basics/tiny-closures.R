# The smallest closures: constant, symbol and NULL bodies. Their calls
# now compile and inline, so argument forcing, defaults, missing-argument
# errors, visibility and promises must behave exactly as before.
f1 <- function() 1
f2 <- function(x) x
f3 <- function() NULL
f4 <- function(x, y) x
f5 <- function() "v"
mk <- function(k) function() k
h <- mk(3)
cat(f1(), "\n")
cat(f2(10), "\n")
cat(f2(x = 11), "\n")
cat(f3(), "\n")
cat(f5(), "\n")
cat(f4(1, 2), "\n")
cat(h(), "\n")
k <- 5
cat(h(), "\n")
print(withVisible(f2(1)))
# Lazy arguments: a formal the body never reads is never evaluated.
g1 <- function(x) 1
cat(g1(stop("never evaluated")), "\n")
# ... but one the body reads is, with its error.
r <- tryCatch(f2(stop("boom")), error = function(e) conditionMessage(e))
cat(r, "\n")
# Defaults still run lazily in the closure's frame.
g2 <- function(x) x
g3 <- function(x = 1 + 2) x
cat(g3(), "\n")
# Missing argument to an identity closure: R's error, verbatim.
r <- tryCatch(g2(), error = function(e) conditionMessage(e))
cat(r, "\n")
# ... and to one of several formals.
r <- tryCatch(f4(, 2), error = function(e) conditionMessage(e))
cat(r, "\n")
# Promises keep their expressions for substitute().
cat(deparse(substitute(f2(a + b))), "\n")
sub <- function(q) substitute(q)
cat(deparse(sub(a + b)), "\n")
# Identity through the apply family.
cat(unlist(lapply(1:3, f2)), "\n")
cat(vapply(list(1L, 2L), f2, 0L), "\n")
cat(mapply(f4, 1:2, 3:4), "\n")
# A symbol body reading an enclosing binding sees reassignments.
mk2 <- function() y
y <- 1
cat(mk2(), "\n")
y <- 2
cat(mk2(), "\n")
# Constant and NULL bodies through Map and Filter.
cat(unlist(Map(function(x) x, 1:3)), "\n")
cat(length(Filter(function(x) FALSE, 1:3)), "\n")
# Strings and logicals as constant bodies.
c1 <- function() "const"
c2 <- function() TRUE
cat(c1(), c2(), "\n")
# Calls in tail position of a tiny closure keep their frames.
frame_name <- function() as.character(sys.call()[[1]])
wrap <- function() frame_name()
cat(wrap(), "\n")
# Compiled loop over tiny closures.
s <- 0
for (i in 1:100) s <- s + f1() + f2(i) + f4(i, 2) + (is.null(f3()) && nchar(f5()) == 1)
cat(s, "\n")
