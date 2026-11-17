# cmp.R's code for the constructs it inlines (docs/g3.md §9.1): control
# flow, subsetting and complex assignment, arithmetic and math, logical
# operators, missing() and on.exit, closures, `...`, switch and tryCatch,
# compiled from source on robigo-rt and disassembled as GNU R does.
local(compiler::disassemble(compiler::cmpfun(function(x) {
  y <- x[[1]]
  x$a <- 2
  z <- switch(y, a = 1, 2)
  repeat break
  while (FALSE) next
  tryCatch(1, error = function(e) 2)
})))
local(compiler::disassemble(compiler::cmpfun(function(x, ...) {
  if (missing(x)) return(NULL)
  on.exit(NULL)
  x[1] <- 2L
  names(x)[2] <- "b"
  is.null(x) && length(x) > 1 || !TRUE
})))
local(compiler::disassemble(compiler::cmpfun(function(a) {
  b <- a %% 2L
  c <- -a
  d <- a^2
  e <- sqrt(a)
  f <- exp(a)
  g <- c(a, 1)
  h <- seq_along(a)
  vapply(a, function(i) i, 0)
})))
local(compiler::disassemble(compiler::cmpfun(function(x, i, j) {
  x[i, j] <- 0
  x[[i]] <- x[[j]]
  y <- x[i]
  attr(x, "k") <- 1
  x
})))
local(compiler::disassemble(compiler::cmpfun(function(n) {
  s <- 0
  i <- 0L
  while (i < n) {
    i <- i + 1L
    if (i %% 2L == 0L) next
    s <- s + i
  }
  s
})))
invisible(NULL)
