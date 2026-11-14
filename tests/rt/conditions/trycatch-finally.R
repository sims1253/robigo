tryCatch({ cat("body\n"); 1 }, finally = cat("finally\n"))
tryCatch(stop("x"), error = function(e) cat("handler\n"), finally = cat("finally\n"))
g <- function() {
  on.exit(cat("on.exit of g\n"))
  stop("from g")
}
tryCatch(g(), error = function(e) cat("caught:", conditionMessage(e), "\n"))
