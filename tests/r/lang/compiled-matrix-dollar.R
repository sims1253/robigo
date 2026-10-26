# m[i, j], m[i, j] <- v and x$name <- v in compiled code
fill <- function(n) { m <- matrix(0, n, n); for (a in 1:n) for (b in 1:n) m[a, b] <- a * 10 + b; m }
fill(3)
rd <- function(m) { s <- 0; for (a in 1:nrow(m)) for (b in 1:ncol(m)) s <- s + m[a, b]; s }
rd(fill(4)); rd(matrix(1:6, 2))
m <- matrix(1:4, 2, dimnames = list(c("a","b"), c("x","y"))); g <- function(m) m[1, 2]; g(m); typeof(g(m)); names(g(m))
ty <- function() { m <- matrix(1:4, 2); m[1, 1] <- 2.5; m }
ty()
ty2 <- function() { m <- matrix(c(1.5, 2, 3, 4), 2); m[2, 2] <- 7L; m[1, 2] <- TRUE; m }
ty2()
oob <- function() { m <- matrix(1:4, 2); tryCatch(m[3, 1], error = function(e) conditionMessage(e)) }
oob()
oob2 <- function() { m <- matrix(1:4, 2); m[3, 1] <- 9L; m }
tryCatch(oob2(), error = function(e) print(conditionMessage(e)))
sh <- function() { m <- matrix(1:4, 2); m2 <- m; m2[1, 1] <- 100L; list(m, m2) }
sh()
dfm <- function() { d <- data.frame(a = 1:2, b = 3:4); d[2, 2] <- 99L; d[1, 1] }
dfm()
strm <- function() { m <- matrix(letters[1:4], 2); m[2, 1] <- "Z"; c(m[2, 1], m[1, 2]) }
strm()
na <- function() { m <- matrix(1:4, 2); m[NA, 1] }
na()
arg <- function(m) { m[1, 1] <- 0L; m }
mm <- matrix(1:4, 2); arg(mm); mm
lst <- function() { l <- list(a = 1, b = "x", c = NULL); for (k in 1:5) l$a <- l$a + 1; l$b <- c(l$b, "y"); l }
lst()
del <- function() { l <- list(a = 1, b = 2); l$a <- NULL; l$z <- 3; l }
del()
self <- function() { l <- list(a = 1); l$s <- l; l$a <- 2; l }
self()
dup <- function() { l <- list(a = 1, a = 2); l$a <- 9; l }
dup()
nn <- function() { l <- list(1, 2); names(l) <- c(NA, "b"); l$b <- 5; l$"NA" <- 6; l }
nn()
envf <- function() { e <- new.env(); e$v <- 1; e$v <- e$v + 1; e$v }
envf()
obj <- function() { o <- structure(list(a = 1), class = "rec"); o$a <- 5; unclass(o) }
obj()
`$<-.rec2` <- function(x, name, value) { cat("method\n"); unclass(x) }
obj2 <- function() { o <- structure(list(a = 1), class = "rec2"); o$a <- 5; o }
obj2()
vec <- function() { v <- c(a = 1, b = 2); tryCatch({ v$a <- 3; v }, error = function(e) conditionMessage(e), warning = function(w) conditionMessage(w)) }
vec()
