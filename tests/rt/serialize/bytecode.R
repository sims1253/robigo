# serialize() of byte code (serialize.c's WriteBC, ReadBC): code made with
# mkCode, its constants holding a language object twice (BCREPDEF, then
# BCREPREF), a call with attributes (ATTRLANGSXP), a pairlist, nested byte
# code and a cell that refers to itself; GNU R's bytes in each format, and
# unserialize() then serialize() gives them again. Byte code of a version
# this R cannot run reads back as its expression.
hex <- function(x) paste(as.character(x), collapse = "")
mk <- function(code, consts) .Internal(mkCode(code, consts))
show <- function(name, v) {
  for (ver in c(2L, 3L)) {
    x <- serialize(v, NULL, version = ver)
    cat(name, " v", ver, " xdr ", hex(x), "\n", sep = "")
    cat(name, " v", ver, " bin ", hex(serialize(v, NULL, xdr = FALSE, version = ver)), "\n", sep = "")
    cat(name, " v", ver, " asc ", hex(serialize(v, NULL, ascii = TRUE, version = ver)), "\n", sep = "")
    y <- unserialize(x)
    cat(name, " v", ver, " type ", typeof(y), " again ", identical(serialize(y, NULL, version = ver), x), "\n", sep = "")
  }
}
# LDCONST 0, RETURN
ret <- c(12L, 16L, 0L, 1L)
e <- quote(x + 1)
show("simple", mk(ret, list(e)))
show("shared", mk(ret, list(e, 42, e, "s")))
a <- quote(f(y))
attr(a, "note") <- "kept"
show("attr", mk(ret, list(a, pairlist(p = 1, q = quote(z)))))
inner <- mk(ret, list(quote(g(w)), e))
show("nested", mk(ret, list(e, inner, list(1, 2))))
loop <- quote(h(k))
loop[[2]] <- loop
show("self", mk(ret, list(loop)))
show("closure", .Internal(bcClose(as.pairlist(alist(x = )), mk(ret, list(e)), globalenv())))
old <- mk(c(8L, 16L, 0L, 1L), list(quote(old(expr))))
print(unserialize(serialize(old, NULL)))
