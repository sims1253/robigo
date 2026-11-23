## ls(sorted = TRUE) orders names with the collation order, not byte order:
## envir.c:do_ls calls sortVector(ans, FALSE), i.e. Scollate (ICU in a
## UTF-8 locale), like sort() on the same strings.
e <- new.env()
e[["a"]] <- 1
e[["B"]] <- 2
e[["_c"]] <- 3
e[["Z"]] <- 4
print(ls(e, sorted = TRUE))
print(ls(e, sorted = TRUE, all.names = TRUE))
print(sort(c("a", "B", "_c", "Z")))
print(ls(e, sorted = FALSE))
