# R: coerce.c:coerceVectorList keeps only the names of a list coerced to
# an atomic type; attrib.c:dimgets stores the coerced dim vector itself,
# names included.
l <- structure(list(a = 1, b = "x"), row.names = 1:2, class = "foo", extra = TRUE)
attributes(as.character(unclass(l)))
x <- 1:2
class(x) <- l
attributes(x)
argv <- list(structure(list(c0 = structure(integer(0), .Label = character(0), class = 'factor')), .Names = 'c0', row.names = character(0), class = structure('integer(0)', .Names = 'c0')), structure(list(c0 = structure(integer(0), .Label = character(0), class = 'factor')), .Names = 'c0', row.names = character(0), class = structure('integer(0)', .Names = 'c0')))
`class<-`(argv[[1]], argv[[2]])
m <- structure(list(1, 2, 3, 4), dim = c(2L, 2L), foo = "bar")
storage.mode(m) <- "double"
attributes(m)
nd <- structure(1:8 / 2, dim = c(a = 2L, b = 2L, c = 2L))
attributes(nd)
attributes(nd[, , 1, drop = FALSE])
attributes(nd[, 2, ])
y <- 1:6
dim(y) <- c(r = 2, s = 3)
attributes(y)
attributes(y[, 2:3])
attributes(t(y))
