# The .Internal generics (as.vector, unlist, lengths) dispatch S4 methods:
# methods sets them on the .Internal (makeBasicFunsList.R, internal=TRUE).
library(methods)

setClass("A", representation(x = "numeric"))
setMethod("as.vector", "A", function(x, mode = "any") x@x * 10)
a <- new("A", x = c(1, 2))
as.vector(a)
(function(v) base::as.vector(v))(a)
as.vector(a, "character")
as(a, "vector")
removeMethod("as.vector", "A")
tryCatch(as.vector(a), error = function(e) conditionMessage(e))

# S4 objects with a data part still fall through to the default.
setClass("N", contains = "numeric")
n <- new("N", c(3, 4))
as.vector(n)
setMethod("as.vector", "N", function(x, mode = "any") rev(x@.Data))
as.vector(n)
base::as.vector(n, "list")

# unlist
setClass("L", representation(items = "list"))
setMethod("unlist", "L", function(x, recursive = TRUE, use.names = TRUE) {
    unlist(x@items, recursive, use.names)
})
l <- new("L", items = list(a = 1, b = 2:3))
unlist(l)
(function(v) base::unlist(v, use.names = FALSE))(l)

# lengths: an S4 method, and the default on an S4 list
setMethod("lengths", "L", function(x, use.names = TRUE) lengths(x@items, use.names))
lengths(l)
base::lengths(l, use.names = FALSE)
setClass("SL", contains = "list")
sl <- new("SL", list(1:3, "a"))
lengths(sl)

# lengths: S3 methods, and the elements' length() and [[ methods
lengths.foo <- function(x, use.names = TRUE) "lengths.foo"
lengths(structure(list(1, 2), class = "foo"))
length.bar <- function(x) 42L
lengths(list(1, structure(1:3, class = "bar"), 1:5))
`[[.baz` <- function(x, i) letters[seq_len(i)]
length.baz <- function(x) 3L
lengths(structure(list(), class = "baz"))
length.big <- function(x) 3e9
lengths(list(structure(1, class = "big"), 1:2))

# dim is kept, names and dimnames only with use.names
m <- matrix(list(1, 1:2, 1:3, "a"), 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
lengths(m)
lengths(m, use.names = FALSE)
lengths(c(a = 1, b = 2))
lengths(NULL)
tryCatch(lengths(list(1), NA), error = function(e) conditionMessage(e))
tryCatch(lengths(quote(f(x))), error = function(e) conditionMessage(e))
