# Writes into a compact sequence (`x <- 1:10`) through each path: the
# sequence is marked not mutable when made (altclasses.c), so every write
# copies it first and leaves the other bindings of it alone. Each path is
# run at top level and in a function the JIT compiles (the VM's in-place
# subassign).
x <- 1:10; y <- x
x[3] <- 100L; print(x); print(y)
x <- 1:10; x[[2]] <- -1L; print(x)
x <- 1:10; x[2:4] <- 0L; print(x)
x <- 1:10; x[x > 5L] <- 0L; print(x)
x <- 1:10; x[12] <- 12L; print(x)
x <- 1:10; x[3] <- 2.5; print(x)
x <- 1:10; x[[3]] <- "a"; print(x)
x <- 1:10; attr(x, "foo") <- "bar"; print(x); print(y)
x <- 1:10; names(x) <- letters[1:10]; print(x)
x <- 1:6; dim(x) <- 2:3; print(dim(x)); print(x[2, 3])
x <- 1:3; class(x) <- "myclass"; print(unclass(x)); print(inherits(x, "myclass"))
x <- 1:10; attributes(x) <- list(a = 1); print(x)
x <- 1:10; is.na(x) <- 2L; print(x)
x <- 1:10; length(x) <- 12L; print(x)
x <- 1:10; storage.mode(x) <- "double"; print(x); print(typeof(x))
x <- 1:3; x$a <- 1L; print(x)
x <- seq_len(5); x[1] <- 9L; print(x); print(seq_len(5))
x <- seq_along(letters[1:4]); x[[4]] <- 0L; print(x)
x <- 3.5:7.5; x[2] <- 0; print(x)
f <- function(n) {
    v <- 1:n
    w <- v
    for (i in seq_along(v)) v[i] <- v[i] * 2L
    v[[1]] <- 0L
    attr(v, "n") <- n
    list(v, w)
}
print(f(5))
g <- function() {
    s <- 1:4
    for (k in 1:3) s[k] <- s[k + 1L]
    s
}
print(g()); print(g())
h <- function(z) { z[1] <- 0L; z }
q <- 1:5
print(h(q)); print(q)
