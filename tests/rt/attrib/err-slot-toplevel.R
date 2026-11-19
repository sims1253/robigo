# `@` on a non-object at top level: error() has no call to report.
x <- structure(1, class = "foo")
`@.foo` <- function(object, name) name
x@ok
1@a
