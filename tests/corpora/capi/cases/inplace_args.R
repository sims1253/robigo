dyn.load("inplace_args.so")

# C writes into a compact sequence through INTEGER()/REAL() and returns
# it (Rcpp's integer_INTSXP(0:9), numeric_REALSXP(as.numeric(0:9))):
# the sequence expands and the result is what C wrote.
.Call("capi_twice_int", 0:9)
.Call("capi_twice_real", as.numeric(0:4))
x <- 1:5
y <- .Call("capi_twice_int", x)
y
identical(x, y)
x + 0L

# two bindings of one sequence see the write; a duplicate C writes into
# leaves its original alone
x <- 1:5
alias <- x
invisible(.Call("capi_twice_int", x))
x
alias
x <- 1:5
alias <- x
d <- .Call("capi_twice_dup", x)
d
x
alias

# C writes into a list's sequence before and after R code (with a native
# call of its own) reads the list it bound
l <- list(1:5)
r <- .Call("capi_bind_write_eval", l,
           quote({ .Call("capi_twice_int", 1:3); bound_l[[1]] }), environment())
r
bound_l[[1]]
l[[1]]

# the same through a pointer from DATAPTR_RO, on an argument C keeps in
# a preserved list meanwhile (Rcpp's IntegerVector::fill)
.Call("capi_fill_ro", 1:10, 10L)

# attributes C sets on a compact sequence (Rcpp's y.names() = names)
names(.Call("capi_set_names", 1:2, c("foo", "bar")))
z <- .Call("capi_set_names", 3:5, c("a", "b", "c"))
z
z[2]
x <- 1:3
invisible(.Call("capi_set_names", x, c("p", "q", "r")))
x

# a character vector C keeps and rewrites through DATAPTR_RO's pointer
# (Rcpp's CharacterVector::sort)
ch <- c(A = "d", B = "b", C = "a")
.Call("capi_rev_str_ro", ch)
ch
.Call("capi_rev_str_ro", c("x", "y", "z", "w"))

# a write that compares equal but is not the same value: -0 over 0
x <- as.numeric(0:2)
invisible(.Call("capi_negzero_real", x))
1 / x
