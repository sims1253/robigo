# x[i] and x[[i]] with index expressions that may run code (compiled code)
side <- function(tag, v) { cat("eval", tag, "\n"); v }
f1 <- function(x) x[side("i", 2)]
f1(c(10, 20, 30))
f2 <- function(x) x[[side("i", 2L)]]
f2(list(a = 1, b = "two"))
f3 <- function() side("x", c(a = 1, b = 2, c = 3))[side("i", c("c", "a"))]
f3()
f4 <- function(x) x[x > 2]
f4(1:5)
f5 <- function(x) x[order(-x)]
f5(c(3, 1, 2))
f6 <- function(x) x[-length(x)]
f6(letters[1:4])
f7 <- function(x, n) x[seq_len(n)]
f7(c(a = 1, b = 2, c = 3), 2)
f7(1:3, 0)
reassign <- function() { x <- 1:3; x[{ x <- 100:103; 2 }] }
reassign()
`[.myc` <- function(x, i) { cat("method [ with", i, "\n"); unclass(x)[i] * 10 }
`[[.myc` <- function(x, i) { cat("method [[\n"); "from method" }
obj <- structure(c(1, 2, 3), class = "myc")
g1 <- function(o) o[side("idx", 3)]
g1(obj)
g2 <- function(o) o[[side("never", 1)]]
g2(obj)
lazy <- function(o, i) o[[i]]
lazy(obj, stop("not evaluated"))
fac <- factor(c("u", "v", "u"))
g3 <- function(z) z[which(z == "u")]
g3(fac)
df <- data.frame(p = 1:3, q = c("a", "b", "c"))
g4 <- function(d, col) d[[paste0(col, "")]]
g4(df, "q")
g5 <- function(d) d[d$p > 1, ]
g5(df)
e <- new.env(); assign("k", 42, e)
g6 <- function(env, nm) env[[paste0(nm)]]
g6(e, "k")
g7 <- function(x) x[[as.integer(side("big", 10))]]
tryCatch(g7(1:3), error = function(err) print(conditionMessage(err)))
g8 <- function(x) x[[sum(1, 1)]]
tryCatch(g8(sum), error = function(err) print(conditionMessage(err)))
g9 <- function(x) x[nchar("ab")]
g9(NULL)
g10 <- function(x) x[[1 + 0]]
g10(list(list(1)))
missing_x <- function(x) x[[sum(1)]]
tryCatch(missing_x(), error = function(err) print(conditionMessage(err)))
vis <- function(x) invisible(x[nchar("a")])
vis(1:3)
withVisible((function(x) x[[nchar("a")]])(list(5)))
loop <- function(x) { s <- 0; for (k in seq_along(x)) s <- s + x[[k + 0L]] * x[length(x) - k + 1L]; s }
loop(c(1, 2, 3, 4))
nested <- function(m) m[[side("outer", 1)]][[side("inner", 2)]]
nested(list(list("a", "b")))
local({
  `[` <- function(x, i) "shadowed"
  h <- function(v) v[nchar("a")]
  print(h(1:3))
})
outer_fn <- function(k) inner_fn(k)
inner_fn <- function(i) (11:13)[i]
outer_fn()
inner2 <- function(i) (11:13)[[i]]
tryCatch(inner2(), error = function(err) print(conditionMessage(err)))
paren <- function(i) (11:13)[(i)]
tryCatch(paren(), error = function(err) print(conditionMessage(err)))
