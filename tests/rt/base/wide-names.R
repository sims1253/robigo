# INTERIM(L5): the wide base (crates/robigo-rt/src/base.rs) binds every name
# GNU R 4.6.2 binds in base: each a closure, a value or a primitive (or the
# stub of one robigo-rt does not implement yet). Listing them forces none.
x <- ls(baseenv(), all.names = TRUE)
cat(length(x), "\n")
cat(sort(x, method = "radix"), sep = "\n")
