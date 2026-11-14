# `class<-` (R_set_class), `oldClass<-` (classgets) and `unclass`
x <- c(a = 1L, b = 2L)
class(x) <- "numeric"
x
typeof(x)
class(x) <- "character"
x
class(x) <- "list"
x
y <- c("1", "a")
class(y) <- "numeric"
y
z <- c(TRUE, NA)
class(z) <- "integer"
z
m <- 1:6
dim(m) <- 2:3
class(m) <- "matrix"
attributes(m)
class(m) <- c("matrix", "array")
attributes(m)
class(m) <- "array"
attributes(m)
class(m) <- c("foo", "bar")
oldClass(m)
class(m)
inherits(m, "bar")
class(m) <- character(0)
oldClass(m)
o <- 1:3
oldClass(o) <- "bar"
oldClass(o)
inherits(o, "bar")
unclass(o)
oldClass(o) <- character(0)
attributes(o)
oldClass(o) <- c("a", "b")
inherits(o, "b", which = TRUE)
unclass(o)
oldClass(o) <- NULL
attributes(o)
e <- quote(a)
class(e) <- "name"
e
u <- unclass(1:2)
u
f <- 1L
class(f) <- "factor"
oldClass(f)
