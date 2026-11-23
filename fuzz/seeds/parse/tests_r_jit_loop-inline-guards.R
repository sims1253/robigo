# A helper recreated per call, run inline by a native loop: its operators'
# bindings are checked once per environment, and again when they change.
outer <- function(n, rebind = FALSE) {
  h <- function(i, j) 1 / ((i + j) * (i + j + 1) / 2 + i + 1)
  if (rebind) `+` <- function(a, b) base::`+`(a, b) * 2
  s <- 0
  for (k in 1:3) {
    v <- double(n)
    for (j in 0:(n - 1)) v[[j + 1]] <- h(k, j)
    s <- s + sum(v)
  }
  s
}
for (i in 1:5) print(outer(40))
print(outer(40, TRUE))
print(outer(40))
`*` <- function(a, b) base::`*`(a, b) + 1
print(outer(40))
rm(`*`)
print(outer(40))
