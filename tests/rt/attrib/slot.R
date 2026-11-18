# `@` (attrib.c do_AT) and `@<-` (do_attrgets) without the methods package.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  print(r)
  if (inherits(r, "error")) print(conditionCall(r))
  invisible()
}
# not an object: formal classes need methods
show(1@a)
show(list()@a)
show(NULL@a)
show(1@1)
f <- function() 1@a
show(f())
# an S3 object dispatches on `@`, the name made a string
x <- structure(1, class = "foo")
`@.foo` <- function(object, name) paste("slot", name, class(name))
x@bar
x@"baz"
`@`(x, "qux")
# no method: an error after the dispatch
y <- structure(1, class = "bar")
show(y@b)
show(y@"b")
show(y@.Data)
# the name's checks (fixSubset3Args) for an object only
show(`@`(y, 1))
show(`@`(y, c("a", "b")))
show(`@`(y, NULL))
show(`@`(1, NULL))
# arity
show(`@`(y))
show(`@`(1, 2, 3))
# `@<-` on an S3 object dispatches on `@<-`, the name a string
`@<-.foo` <- function(object, name, value) {
  attr(object, name) <- paste(class(name), value)
  object
}
x@slt <- 3
attr(x, "slt")
x <- `@<-`(x, "s2", 4)
attr(x, "s2")
# no method: methods' checkAtAssignment, after initMethodDispatch()
z <- 1
show(z@a <- 2)
show(`@<-`(z, a, 2))
show(`@<-`(z, "a", 2))
show(`@<-`(y, a, 2))
# the name's checks come first
show(`@<-`(z, 1, 2))
show(`@<-`(z, c("a", "b"), 2))
show(`@<-`(z, NULL, 2))
show(`@<-`(z, a))
show(`@<-`(z, a, 2, 3))
