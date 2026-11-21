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
# unserialize installs a UTF-8 name it has not seen: the bytes of a
# serialized `zq\u00e9` with its first byte changed name a new symbol
b <- serialize(as.name("zq\u00e9"), NULL)
i <- which(b == charToRaw("z"))
i <- i[length(i)]
b[i] <- charToRaw("y")
y <- unserialize(b)
print(as.character(y) == "yq\u00e9")
print(Encoding(as.character(y)))
