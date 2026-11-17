# `[[<-` on a call or pairlist (subassign.c's SimpleListAssign, listRemove
# and GetOneIndex): replace, append past the end (tagged by a character
# subscript), remove with NULL, recurse into a call held in a list, and
# GetOneIndex's error for an empty subscript.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
q <- quote(f(x, y))
q[[2]] <- as.name("z")
print(q)
q[[4]] <- 1
print(q)
q[["n"]] <- 2
print(q)
q[[3]] <- NULL
print(q)
print(typeof(q))
p <- pairlist(a = 1, b = 2)
p[["b"]] <- 3
p[["c"]] <- "new"
print(as.list(p))
p[[1]] <- NULL
print(as.list(p))
print(typeof(p))
l <- list(a = quote(g(x, 1)))
l[[c(1, 3)]] <- 2
print(l$a)
l[[c(1, 2)]] <- NULL
print(l$a)
print(err({p[[integer(0)]] <- 2; p}))
print(err({q[[integer(0)]] <- NULL; q}))
