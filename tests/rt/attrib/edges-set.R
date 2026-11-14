# setter edges: names<- evaluates as.character(value) (a symbol is looked up),
# attributes on symbols and environments, a factor is not integer row names
a <- c("p", "q")
x <- 1:2
names(x) <- quote(a)
x
s <- quote(b)
attributes(s) <- NULL
s
e <- new.env()
class(e) <- "foo"
oldClass(e)
class(e) <- NULL
oldClass(e)
attributes(e) <- list(z = 1)
attr(e, "z")
attributes(e) <- NULL
attributes(e)
y <- 1:3
names(y) <- 1:3
attributes(y)
v <- list(a = 1:3)
f <- 1:3
oldClass(f) <- "factor"
attr(f, "levels") <- c("a", "b", "c")
attr(v, "row.names") <- f
