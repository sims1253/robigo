# mapply calls FUN as f(dots[[1L]][[k]], dots[[2L]][[k]], <MoreArgs>),
# as GNU R's mapply.c builds it: that call shows in errors, warnings,
# sys.call() and match.call()
f <- function(x, y) stop("boom ", x)
r <- tryCatch(mapply(f, 1:2, 3:4), error = function(e) e)
print(conditionCall(r))
print(conditionMessage(r))
h <- function(x, y) { warning("ww"); x }
withCallingHandlers(mapply(h, 1, 2), warning = function(w) {
  print(conditionCall(w))
  invokeRestart("muffleWarning")
})
g <- function(x, y) print(sys.call())
invisible(mapply(g, 1:2, 3:4))
invisible(mapply(function(x, y) print(sys.call()), 1:2, c(a = 3, b = 4)))
invisible(mapply(function(...) print(match.call()), 1:2, y = 3:4,
                 MoreArgs = list(z = 5)))
invisible(mapply(function(x, y) print(sys.call()), x = 1, y = "a"))
print(mapply(function(x, ...) nargs(), 1:2, MoreArgs = list(3, 4)))
k <- function() mapply(function(x) sys.call(-1), 1)
print(k())
print(mapply(function(x) deparse(substitute(x)), 1:2))
print(mapply(paste, 1:2, c("a", "b")))
print(mapply(rep, 1:3, 3:1))
print(mapply(function(x, y) x + y, c(a = 1, b = 2), 3:4))
try(mapply(function(x) stop("e1"), 1))
