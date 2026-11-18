g <- function(n) { d <<- n; g(n + 1) }
r <- tryCatch(g(1), error = function(e) conditionMessage(e))
cat(d, "\n"); cat(r, "\n")
