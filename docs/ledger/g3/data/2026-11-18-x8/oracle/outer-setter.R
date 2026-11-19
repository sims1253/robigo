# X8 premise (2026-11-18-x8-premise.md): GNU R keeps the old vector intact
# when code runs or an error is raised after EnlargeVector.
m <- function() {
  x <- list(a = as.numeric(1:3)); class(x) <- "cls"
  `$<-.cls` <- function(x, name, value) { cat("outer setter sees x$a =", unclass(x)$a, "\n"); y <- unclass(x); y[[name]] <- value; class(y) <- "cls"; y }
  environment(`$<-.cls`) <- globalenv(); assign("$<-.cls", `$<-.cls`, globalenv())
  x$a[5] <- 9
  cat("after:", unclass(x)$a, "\n")
}
m()
