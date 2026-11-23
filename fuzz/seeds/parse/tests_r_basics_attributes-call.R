# attributes() of a call does not include its argument tags (attrib.c:
# do_attributes reports names only for a pairlist), while names() does.
x <- quote(foo(bar = 1, 2))
attributes(x)
names(x)
attributes(as.pairlist(list(a = 1, 2)))
y <- x
attr(y, "note") <- "kept"
attributes(y)
attributes(quote(f()))
attributes(quote(f(a = 1)[[1]]))
is.null(attributes(call("g", z = 3)))
# attributes<- keeps a call's tags too (it clears only a pairlist's)
m <- quote(h(x = x))
attributes(m) <- NULL
m
attributes(m) <- list(foo = 1)
m
p <- as.pairlist(list(a = 1))
attributes(p) <- NULL
names(p)
