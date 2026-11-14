# Complex assignment in compiled code (cmp.R's cmpComplexAssign, bcEval's
# STARTASSIGN ... ENDASSIGN): the object reaches getters and setters as an
# argument (`*tmp*` is never bound), the calls are the compiler's (the
# right-hand side's expression, `*vtmp*` for an inner setter), and error()
# reports the setter's call as an assignment. Each function has a loop, so
# GNU R compiles it at its first call.
`tag<-` <- function(x, value) {
  print(sys.call())
  print(.Internal(getBindingType(quote(x), environment())))
  print(.Internal(getBindingType(quote(value), environment())))
  print(.Internal(exists("*tmp*", parent.frame(), "any", FALSE)))
  attr(x, "tag") <- value
  x
}
tag <- function(x) attr(x, "tag")
f <- function() { for (i in 1) NULL; x <- 1:3; tag(x) <- 2 + 3; x }
print(f())
g <- function() { for (i in 1) NULL; x <- 1:3; tag(x)[2] <- 9; x }
print(g())
h <- function() { for (i in 1) NULL; x <- c(a = 1, b = 2); names(x)[2] <- "z"; x }
print(h())
l <- function() { for (i in 1) NULL; x <- list(a = list(b = 1)); x$a$b <- 2; x$a$c <- 3; x[["d"]] <- 4; x }
print(l())
s <- function() { for (i in 1) NULL; v <- 1:5; v[2:3] <- c(20L, 30L); v[[5]] <- 50L; v }
print(s())
sup <- function() { y <- 1:3; k <- function() { for (i in 1) NULL; y[2] <<- 7L }; k(); y }
print(sup())
e <- function() { for (i in 1) NULL; en <- new.env(); en$a <- 1; en[["b"]] <- 2; c(en$a, en$b) }
print(e())
# S3 methods of `[<-` and `$<-`
`[<-.obj` <- function(x, i, value) { print(sys.call()); y <- unclass(x); y[i] <- value * 10; structure(y, class = "obj") }
`$<-.obj` <- function(x, name, value) { print(sys.call()); x }
o <- function() { for (i in 1) NULL; z <- structure(1:3, class = "obj"); z[2] <- 5L; z$q <- 1; unclass(z) }
print(o())
# errors
bad <- function() { for (i in 1) NULL; x <- 1:3; x[[5, 2]] <- 1; x }
tryCatch(bad(), error = function(e) print(conditionCall(e)))
nofun <- function() { for (i in 1) NULL; x <- 1; foo(x) <- 2; x }
tryCatch(nofun(), error = function(e) { print(conditionCall(e)); print(conditionMessage(e)) })
`stops<-` <- function(x, value) stop("in stops<-")
st <- function() { for (i in 1) NULL; x <- 1; stops(x) <- 1; x }
tryCatch(st(), error = function(e) print(conditionCall(e)))
