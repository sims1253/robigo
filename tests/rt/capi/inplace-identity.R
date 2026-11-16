# A native observer of R's in-place writes (docs/g3.md §15.3 E5): C reads
# an object's address, as `.Internal(inspect())` prints it, before and
# after each write. GNU R writes an object that only its binding
# references in place and copies a shared one; robigo's production answers
# are GNU R's. ROBIGO_NO_INPLACE=1 copies at every write, so this
# recording holds in production only (the .requires: inplace-identity).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
x <- c(1, 2, 3)
a <- .Call("rt_addr", x); x[2] <- 20; print(.Call("rt_addr", x) == a)
a <- .Call("rt_addr", x); x[[3]] <- 30; print(.Call("rt_addr", x) == a)
a <- .Call("rt_addr", x); names(x) <- c("a", "b", "c"); print(.Call("rt_addr", x) == a)
a <- .Call("rt_addr", x); attr(x, "u") <- 1; print(.Call("rt_addr", x) == a)
# shared: the write copies, the other binding keeps the old object
y <- x
a <- .Call("rt_addr", x); x[1] <- 10; print(.Call("rt_addr", x) == a)
print(.Call("rt_addr", y) == a)
print(x)
print(y)
# nested: the outer list and the inner one written in place
l <- list(a = 1, b = list(c = 2))
a <- .Call("rt_addr", l); l$b$c <- 3; print(.Call("rt_addr", l) == a)
print(l$b$c)
# a local, in a closure's first call (the AST) and later ones (byte code)
f <- function() {
  v <- c(1, 2)
  a <- .Call("rt_addr", v)
  v[1] <- 5
  c(.Call("rt_addr", v) == a, v)
}
print(f())
print(f())
print(f())
# the observer that holds the old object while it compares (rt_assign_copies,
# for writes after which the old object would die: #709): TRUE when the
# write copies. Right after a top-level assignment, .Last.value holds the
# value without a count (tests/rt/toplevel/last-value-counts.R), so the
# first write is in place.
x <- c(1, 2, 3)
print(.Call("rt_assign_copies", quote(x), quote(x[1] <- 5), environment()))
y <- x
print(.Call("rt_assign_copies", quote(x), quote(x[1] <- 6), environment()))
print(c(x[1], y[1]))
