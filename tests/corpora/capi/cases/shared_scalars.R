dyn.load("shared_scalars.so")

# a loop index written by C (purrr's map), in a closure
f <- function() { i <- 0L; .Call("capi_ss_set_int", i, 7L); invisible() }
f()
print(0L)
print(c(a = 0L, b = 1L))
g <- function() { k <- 5L; .Call("capi_ss_set_int", k, 99L); invisible() }
g()
print(5L)
# at top level too
i <- 0L
invisible(.Call("capi_ss_set_int", i, 3L))
print(0L)

x <- TRUE
invisible(.Call("capi_ss_set_lgl", x))
print(TRUE)
print(c(TRUE, NA))

y <- 1L
invisible(.Call("capi_ss_set_attr", y))
print(attributes(1L))
