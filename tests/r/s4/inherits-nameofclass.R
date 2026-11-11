# inherits(x, what) with a classed, non-character 'what' asks
# base::nameOfClass(what), as objects.c:do_inherits does.
ClassX <- structure(list(), name = "ClassX",
                    class = c("S3pp_class", "S3pp_object"))
classx_instance <- structure(list(), class = c("ClassX", "S3pp_object"))
nameOfClass.S3pp_class <- function(x) attr(x, "name", TRUE)
nameOfClass.foo <- function(x) "bar"
print(inherits(classx_instance, "ClassX"))
print(inherits(classx_instance, ClassX))
print(inherits(classx_instance, ClassX, which = TRUE))
print(inherits(1, ClassX))
# a character 'what' keeps its class
print(inherits(1, structure("numeric", class = "foo")))
# the class is not evaluated in calling nameOfClass
print(inherits(1, structure(quote(stop("should not be evaluated")), class = "foo")))
print(inherits(structure(1, class = "bar"), structure(quote(stop("no")), class = "foo")))
# a NULL name falls back to the plain check, which rejects 'what'
print(tryCatch(inherits(1, structure(list(), class = "nul")),
               error = function(e) conditionMessage(e)))
# nameOfClass sees the caller's environment
f <- function() {
  nameOfClass.local <- function(x) "numeric"
  inherits(1, structure(list(), class = "local"))
}
print(tryCatch(f(), error = function(e) conditionMessage(e)))
# the S4 path: class(x) is S4 extends()
library(methods)
setClass("P", representation(a = "numeric"))
setClass("Q", contains = "P")
print(inherits(new("Q"), structure(list(), class = "S3pp_class", name = "P")))
# S3 dispatch on `@` (registerS3method)
x <- structure(list(), class = "foo", prop1 = "prop1val")
registerS3method("@", "foo", function(x, name) {
  cat(sprintf("called `@.foo`(x = %s, name = '%s')\n", deparse1(substitute(x), "\n"), name))
  attr(x, name, TRUE)
})
print(x@prop1)
print(tryCatch(letters@foo, error = function(e) conditionMessage(e)))
