# A symbol's PRINTNAME is the CHARSXP installTrChar was given when no
# translation is needed (names.c:installNoTrChar): a UTF-8 name keeps its
# mark; a latin1 name is translated, and names the same symbol.
e <- new.env()
u <- "café"
l <- "caf\xe9"; Encoding(l) <- "latin1"
.Internal(assign(u, 1, e, FALSE))
.Internal(assign(l, 2, e, FALSE))
.Internal(assign("plain", 3, e, FALSE))
n <- sort(ls(e))
print(Encoding(n))
print(get(u, e))
print(Encoding(names(as.list(e, sorted = TRUE))))
print(Encoding(as.character(as.name(u))))
print(identical(as.name(u), as.name(l)))
x <- list(1, 2); names(x) <- c("cafè", "plain2")
y <- structure(1, names = "n"); attr(y, "àb") <- 1
print(Encoding(names(attributes(y))))
s <- serialize(n, NULL, ascii = TRUE)
r <- strsplit(rawToChar(s), "\n")[[1]]
print(r[-(1:7)])
