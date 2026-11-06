dyn.load("zerocopy.so")

# a counter C writes into, read by each callback (top level: interpreted)
i <- 500L
print(unlist(.Call("capi_zc_loop", quote(identity(i * 10L)), environment(), i)))
print(i)

# found by name, written before each eval
e <- new.env()
assign("g", c(1, 2), envir = e)
print(.Call("capi_zc_found", quote(g), quote(g[1] + 1), e))
print(get("g", envir = e))

# C hands an argument back to R: the same object, so a write is seen
x <- c(1L, 2L, 3L)
y <- .Call("capi_zc_reeval", quote(identity), x, environment())
print(identical(x, y))

# an element of a list argument
l <- list(c(1L, 2L), "a")
.Call("capi_zc_elt", l)
print(l[[1]])

# copy-on-modify: a later change in R does not reach the alias C wrote to
z <- c(5, 6)
w <- z
z[1] <- 0
.Call("capi_zc_found", quote(w), quote(w[1]), environment())
print(z)

# an attribute C set on an argument
a <- c(1.5, 2.5)
.Call("capi_zc_attr", a)
print(attributes(a))

# the shared constants are never written: TRUE and small integers stay
t <- TRUE
print(c(0L, 1L, TRUE))

# C changes a fresh logical's type in place and returns it
y <- .Call("capi_zc_retype", duplicated(c("a", "b", "a")))
print(typeof(y))
print(y)
f <- utils::type.convert(c("abc", NA, "def"), as.is = FALSE)
print(typeof(f))
print(unclass(f))
