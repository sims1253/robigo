# mapply: recycling, names, MoreArgs, SIMPLIFY, zero lengths, warnings
mapply(function(x, y) x + y, 1:3, 4:6)
mapply(rep, 1:4, 4:1)
mapply(rep, times = 1:4, x = 4:1)
mapply(rep, times = 1:4, MoreArgs = list(x = 42))
mapply(function(x, y) seq_len(x) + y, c(a = 1, b = 2, c = 3), c(A = 10, B = 0, C = -10))
mapply(function(x, y) paste(x, y), c("a", "b"), c("x", "y", "z", "w"))
mapply(function(x, y) x, 1:3, 1:2)
mapply(function(x, y) x, list(), 1:3)
mapply(function(x, y) x, 1:3, NULL)
mapply(function(...) sum(...), 1:3, 4:6, 7:9)
mapply(function(x, y) list(x, y), 1:2, c("a", "b"), SIMPLIFY = FALSE)
mapply(function(x, y) x * y, 1:3, 2, USE.NAMES = FALSE)
mapply(function(x) x, c(p = 1, q = 2))
mapply(function(x) x, c("u", "v"))
mapply(function(x, k) x^k, 2:4, MoreArgs = list(k = 2))
Map(`+`, 1:3, 4:6)
Map(function(x, y) x:y, 1:2, 3:4)
# the call FUN sees: its arguments are promises of dots[[j]][[i]]
mapply(function(x, y) sys.call(), 1:2, 3:4)
mapply(function(x) substitute(x), 1:3)
# a classed argument's length() method
length.three <- function(x) 3L
`[[.three` <- function(x, i) i * 10
mapply(function(a, b) a + b, structure(list(), class = "three"), 1:3)
# MoreArgs as a pairlist: accepted, then not used (do_mapply)
mapply(function(x, y) x + y, 1:2, MoreArgs = pairlist(y = 100))
