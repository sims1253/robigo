dyn.load("dots_write.so")

f <- function(...) {
  .Call("capi_dots_write_frame", environment())
  list(...)
}
str(f(1, 2))
str(f("x", y = "y", 3))

g <- function(...) environment()
e <- g(10, 20)
d <- as.list.environment(e, all.names = TRUE)[["..."]]
invisible(.Call("capi_dots_write_arg", d))
str(eval(quote(list(...)), e))

h <- function(...) .Call("capi_dots_same", environment())
print(h(1, 2))
