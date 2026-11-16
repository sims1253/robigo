# A method's argument match, memoized by the method and the generic's
# actuals (their tags and missing ones; robigo's plans for methods called
# with their generics' arguments): each call of the same method with
# other tags, a missing actual, a partial match (and its warning), and a
# method redefined between calls, matched afresh as GNU R matches.
g <- function(x, ...) UseMethod("g")
g.foo <- function(x, a = "da", b = "db", ...) paste(a, b, ...length())
o <- structure(1, class = "foo")
for (i in 1:2) {
  print(g(o))
  print(g(o, 1))
  print(g(o, 1, 2))
  print(g(o, b = 1))
  print(g(o, b = 1, 2))
  print(g(o, 2, b = 1))
  print(g(o, b = 1, a = 2, 3))
  print(g(o, , 2))
  print(g(o, c = 3))
}
# a partial match, with its warning, again
g2 <- function(x, ...) UseMethod("g2")
g2.foo <- function(x, alpha = 1, ...) alpha
old <- options(warnPartialMatchArgs = TRUE)
for (i in 1:2) print(withCallingHandlers(g2(o, al = 5), warning = function(w) {
  cat("warning:", conditionMessage(w), "\n")
  invokeRestart("muffleWarning")
}))
options(old)
# the method redefined: its new formals matched
g.foo <- function(x, b = "nb", a = "na") paste(a, b)
for (i in 1:2) {
  print(g(o, 1))
  print(g(o, b = 1))
}
# an unused argument, every time
g.foo <- function(x, a) a
for (i in 1:2) print(tryCatch(g(o, 1, 2), error = function(e) conditionMessage(e)))
# more methods than plans kept, called in turn twice
h <- function(x, ...) UseMethod("h")
mk <- function(i) function(x, p = 0, q = 0) p * 100 + q + i
for (i in 1:20) assign(paste0("h.c", i), mk(i))
r <- 0
for (k in 1:2) {
  for (i in 1:20) {
    obj <- structure(1, class = paste0("c", i))
    r <- r + h(obj, q = i) + h(obj, i, i)
  }
}
print(r)
