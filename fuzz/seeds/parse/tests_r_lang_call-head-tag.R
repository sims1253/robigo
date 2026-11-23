# A call's first cell can carry a tag (parseNamespaceFile's useDynLib
# handling relies on names(e[-c(1, 2)])).
e <- quote(useDynLib(pk, .registration = TRUE))
x <- e[-c(1, 2)]
names(x)
names(e[c(3, 1)])
z <- as.call(list(a = quote(f), 1))
names(z)
names(z) <- NULL
names(z)
names(e) <- c("h", "a", "b")
names(e)
e[[1]]
as.list(e[c(2, 3)])
try(as.call(list()))
try(as.call("f"))
as.call(pairlist(a = quote(g), 2))
