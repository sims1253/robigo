# mapply's errors and forcing order
show <- function(expr) tryCatch(expr,
    error = function(e) cat("Error in", deparse(conditionCall(e)), ":", conditionMessage(e), "\n"),
    warning = function(w) cat("Warning in", deparse(conditionCall(w)), ":", conditionMessage(w), "\n"))
show(mapply(function(x) x, 1:3, MoreArgs = 1))
show(mapply(function(x) x, 1:3, MoreArgs = "a"))
show(.Internal(mapply(function(x) x, list(1:2), 5)))
show(mapply(function(x, y) x, 1:3, 1:2))
show(mapply(1, 1:3))
show(mapply(function(x) stop("boom ", x), 1:3))
# all varying arguments are forced before FUN's body runs
g <- function(x, y) { cat("body\n"); 0 }
invisible(mapply(g, c(a = 1), c(b = 2)))
h <- function(x, y) { cat("in h\n"); y }
mapply(h, 1:2, list(quote(a), quote(b)))
# called directly, the call FUN gets reads `dots` from the caller's frame
show(.Internal(mapply(function(x, y) x + y, 1:3, NULL)))
dots <- list(1:3, 4:6)
.Internal(mapply(function(x, y) x + y, dots, NULL))
dots <- list(a = 1:2, 3:4)
.Internal(mapply(function(a, y, z) a + y + z, dots, list(5)))
.Internal(mapply(function(x, y, z) x + y + z, dots, list(5)))
