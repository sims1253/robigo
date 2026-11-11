dyn.load("column_ptr_write.so")

# A list whose columns C reorders in place through DATAPTR_RO
# (data.table's setorder): R sees every column reordered.
df <- list(i = 1:4, d = c(1.5, 2.5, 3.5, 4.5), s = c("a", "b", "c", "d"),
           l = list(1, "two", 3L, NULL))
invisible(.Call("capi_reverse_columns", df))
print(df)

# A column R also holds elsewhere is the same object: it changes too.
s <- c("w", "x", "y", "z")
df2 <- list(s = s)
invisible(.Call("capi_reverse_columns", df2))
df2$s
s
