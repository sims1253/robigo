# A compiled complex assignment's setter call of a special (`[<-`, `[[<-`,
# `$<-`) on an object: bcEval's SETTER_CALL gives the special the
# compiler's call (`*tmp*` in it) and the arguments apart (evaluated
# promises of `*tmp*` and of the value), so a method's sys.call() is that
# call and match.call() matches it (eval.c: SETTER_CALL, R_DispatchOrEvalSP).
# The AST's applydefine shows the same. A loop makes each closure compiled
# at its first call; each closure has a body of its own (the JIT cache).
x <- structure(list(1, 2), class = "foo")
`[<-.foo` <- function(x, ..., value) { print(sys.call()); print(match.call()); x }
`[[<-.foo` <- function(x, i, value) { print(sys.call()); print(match.call()); x }
`$<-.foo` <- function(x, name, value) { print(sys.call()); print(match.call()); x }
a1 <- function(...) { for (i in 1) NULL; x[...] <- 9; invisible() }
a1(1)
a2 <- function(j) { for (i in 1) NULL; x[j, drop = FALSE] <- 8; invisible() }
a2(2)
a3 <- function(k) { for (i in 1) NULL; x[[k]] <- 7; invisible() }
a3(1)
a4 <- function(...) { for (i in 1) NULL; x[[...]] <- 6; invisible() }
a4(2)
a5 <- function() { for (i in 1) NULL; x$nm <- 5; invisible() }
a5()
a6 <- function(v) { for (i in 1) NULL; x[] <- v; invisible() }
a6(4)
# the AST (a body GNU R never compiles: it may call browser())
b1 <- function(...) { if (FALSE) browser(); x[...] <- 9; invisible() }
b1(1)
b3 <- function(k) { if (FALSE) browser(); x[[k]] <- 7; invisible() }
b3(1)
b5 <- function() { if (FALSE) browser(); x$nm <- 5; invisible() }
b5()
# the special's own visibility: none around it (SETTER_CALL calls PRIMFUN
# directly), so return() of the assignment shows what it left
r1 <- function(...) { for (i in 1) NULL; y <- 1:3; invisible(0); return(y[...] <- 5L) }
r1(2)
r2 <- function(...) { for (i in 1) NULL; y <- list(1, 2); invisible(0); return(y[[...]] <- 5) }
r2(1)
r3 <- function(...) { for (i in 1) NULL; invisible(0); return(x[...] <- 3) }
r3(1)
