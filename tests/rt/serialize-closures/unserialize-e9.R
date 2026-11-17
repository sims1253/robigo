# E9 inside unserialize (scripts/rt-fail-alloc.py): an object graph with
# environments (hashed, a closure frame, a cycle), closures, attributes,
# calls and byte code (a shared call, nested code), serialized and read back
# in each format. Every refused allocation
# is retried with the objects built so far kept, so no point of injection
# changes what is printed.
e <- new.env()
assign("a", c(1.5, 2), envir = e)
assign("self", e, envir = e)
u <- (function() {
  p <- "x"
  q <- list(1L, "y")
  environment()
})()
obj <- list(n = c(a = 1, b = 2), s = c("x", NA, "é"), l = list(1L, list(TRUE)), e = e, u = u,
            f = eval(quote(function(z) p), u), q = quote(f(x, y = 2)), m = matrix(c(1, 2, 3, 4), 2),
            bc = .Internal(mkCode(c(12L, 16L, 0L, 1L), list(quote(g(x)), quote(g(x)),
                 .Internal(mkCode(c(12L, 16L, 0L, 1L), list(quote(h(1)))))))))
obj$bc2 <- .Internal(mkCode(c(12L, 16L, 0L, 1L), list(obj$q, obj$q)))
ref <- serialize(obj, NULL)
for (fmt in list(list(), list(xdr = FALSE), list(ascii = TRUE), list(version = 2L))) {
  x <- do.call(serialize, c(list(obj, NULL), fmt))
  y <- unserialize(x)
  print(identical(serialize(y, NULL), ref))
}
print(y$n)
print(y$s)
print(get("a", envir = y$e))
print(identical(get("self", envir = y$e), y$e))
print(y$f(1))
print(sort(ls(y$u)))
print(y$q)
print(as.vector(y$m))
print(dim(y$m))
print(typeof(y$bc))
print(identical(serialize(y$bc2, NULL), serialize(obj$bc2, NULL)))
