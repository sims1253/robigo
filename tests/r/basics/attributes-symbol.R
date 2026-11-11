# Attributes cannot be set on a symbol (attrib.c:installAttrib). class<-
# and attr(, "class")<- reach it through classgets, "row.names" through
# row_names_gets, everything else through attributes<- / attr<-; tsp
# fails earlier, in nrows (util.c:nrows). Removing class or all
# attributes is allowed (classgets strips without installing).
x <- quote(a)
print(try(attributes(x) <- list(class = "foo"), silent = TRUE))
x <- quote(a)
print(try(class(x) <- "foo", silent = TRUE))
x <- quote(a)
print(try(attr(x, "class") <- "foo", silent = TRUE))
x <- quote(a)
print(try(attr(x, "dim") <- 2L, silent = TRUE))
x <- quote(a)
print(try(attr(x, "dimnames") <- list("x"), silent = TRUE))
x <- quote(a)
print(try(attr(x, "comment") <- "hello", silent = TRUE))
x <- quote(a)
print(try(attr(x, "row.names") <- "r1", silent = TRUE))
x <- quote(a)
print(try(attr(x, "tsp") <- c(1, 2, 1), silent = TRUE))
x <- quote(a)
print(try(attr(x, "other") <- 5, silent = TRUE))
x <- quote(a)
print(class(x) <- NULL)
x <- quote(a)
print(attributes(x) <- NULL)
x <- quote(a)
print(try(class(x) <- "factor", silent = TRUE))
x <- quote(a)
print(try(dim(x) <- 2L, silent = TRUE))
x <- quote(a)
print(try(`comment<-`(x, "hi"), silent = TRUE))
x <- quote(a)
print(try(structure(x, class = "foo"), silent = TRUE))
# the symbol itself is untouched everywhere else
print(class(quote(a)))
a <- 1
print(class(a))
print(try(attr(quote(missing_name), "x") <- 1, silent = TRUE))
# a failed replacement keeps the binding exactly as it was
x <- quote(a)
try(class(x) <- "foo")
print(x)
y <- 1
try(attr(y, "tsp") <- c(1, 2, 0))
print(y)
