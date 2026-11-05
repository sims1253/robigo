dyn.load("dots_write.so")

f <- function(...) {
  .Call("capi_dots_write_frame", environment())
  list(...)
}
print(f(1, 2))
print(f("x", y = "y", 3))

g <- function(...) environment()
e <- g(10, 20)
d <- as.list.environment(e, all.names = TRUE)[["..."]]
invisible(.Call("capi_dots_write_arg", d))
print(eval(quote(list(...)), e))

h <- function(...) .Call("capi_dots_same", environment())
print(h(1, 2))

e2 <- new.env()
k <- function(...) {
  .Call("capi_dots_bind", environment(), e2)
  invisible(gc())
  .Call("capi_dots_bound_same", e2)
}
print(k(1, 2))

e3 <- g(10, 20)
l <- as.list.environment(e3, all.names = TRUE)
invisible(.Call("capi_dots_append_lang", l))
print(length(l[["..."]]))
print(eval(quote(list(...)), e3))
