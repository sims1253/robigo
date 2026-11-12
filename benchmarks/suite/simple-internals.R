# Base closures that only wrap an .Internal (nchar, typeof, inherits,
# identical, match, rep_len), which GNU R's compiler inlines as direct
# .Internal calls: in a top-level loop (compiled by the JIT, guarded by
# BASEGUARD), in a closure's body, and in the promised arguments its
# compiled code makes.
n <- 30000
x <- c("ab", "cde")
s <- 0
for (i in 1:n) {
  s <- s + nchar(x[1L]) + (typeof(i) == "integer") + inherits(x, "foo")
  s <- s + identical(i, 3L) + match(i, 1:5, 0L) + length(rep_len(i, 2L))
}
id <- function(a) a
g <- function(v, k) id(nchar(v)) + id(identical(k, 7L)) + nchar(v)
for (i in 1:n) s <- s + g("abcd", i)
cat(s, "\n")
