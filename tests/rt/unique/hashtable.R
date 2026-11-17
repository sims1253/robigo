# unique.c's hash table (HashTableSetup) through each of its users: the
# sized tables, the fixed logical and raw ones, fromLast with
# incomparables, a table large enough to wrap its probes, and nmax's
# "hash table is full" (last: it stops the script).
x <- c(5L, 3L, 5L, NA, 3L, 9L, NA)
.Internal(duplicated(x, FALSE, FALSE, NA))
.Internal(duplicated(x, FALSE, TRUE, NA))
.Internal(duplicated(x, 3L, TRUE, NA))
.Internal(unique(x, NA_integer_, TRUE, NA))
.Internal(anyDuplicated(x, FALSE, TRUE))
.Internal(anyDuplicated(x, c(5L, 3L, NA), FALSE))
attributes(.Internal(duplicated(x, FALSE, FALSE, NA)))
.Internal(unique(as.raw(c(1, 255, 1, 0, 255)), FALSE, FALSE, NA))
.Internal(duplicated(c(TRUE, NA, FALSE, TRUE, NA), FALSE, TRUE, NA))
.Internal(unique(c(1+2i, 1-2i, 1+2i), FALSE, FALSE, NA))
s <- paste0("s", c(1:300, 1:300))
u <- .Internal(unique(s, FALSE, FALSE, NA))
c(length(u), u[c(1, 300)])
sum(.Internal(duplicated(s, FALSE, TRUE, NA)))
match(list(c(1, 2)), list(1, c(1, 2), "a"))
match(c("s7", "s300", "nope"), s)
.Internal(unique(c(1, 2, 3, 1), FALSE, FALSE, 2L))
.Internal(unique(c(1, 2, 3, 4), FALSE, FALSE, 2L))
