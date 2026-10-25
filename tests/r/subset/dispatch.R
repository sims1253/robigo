# S3 dispatch of `[`, `[[`, `$` and their replacement forms.
rec <- list(a = 1, b = 2, c = 3)
class(rec) <- "rec"
`[.rec` <- function(x, i, ...) {
  cat("[.rec called with", nargs(), "args\n")
  structure_rec <- unclass(x)[i]
  class(structure_rec) <- "rec"
  structure_rec
}
`[[.rec` <- function(x, i) {
  cat("[[.rec:", i, "\n")
  unclass(x)[[i]] * 100
}
`$.rec` <- function(x, name) {
  cat("$.rec:", name, "\n")
  unclass(x)[[name]] + 0.5
}
r1 <- rec[c("a", "c")]
class(r1)
names(r1)
rec[["b"]]
rec$c
rec[[2]]
`[<-.rec` <- function(x, i, value) {
  cat("[<-.rec\n")
  y <- unclass(x)
  y[i] <- value
  class(y) <- "rec"
  y
}
`[[<-.rec` <- function(x, i, value) {
  cat("[[<-.rec\n")
  y <- unclass(x)
  y[[i]] <- value * 2
  class(y) <- "rec"
  y
}
`$<-.rec` <- function(x, name, value) {
  cat("$<-.rec:", name, "\n")
  y <- unclass(x)
  y[[name]] <- -value
  class(y) <- "rec"
  y
}
rec["a"] <- 10
rec[["b"]] <- 20
rec$c <- 30
unclass(rec)
rec$d <- 4
names(rec)
# nested replacement goes through the methods
holder <- list(inner = rec)
holder$inner$a <- 7
unclass(holder$inner)$a
# NextMethod to the internal default
`[.keep` <- function(x, ...) {
  y <- NextMethod()
  class(y) <- class(x)
  y
}
k <- c(5, 6, 7)
class(k) <- "keep"
k2 <- k[2:3]
class(k2)
unclass(k2)
`[[.keep` <- function(x, ...) {
  cat("[[.keep\n")
  NextMethod()
}
k[[3]]
`[<-.keep` <- function(x, i, value) {
  cat("[<-.keep\n")
  NextMethod()
}
k[1] <- 0
unclass(k)
class(k)
# classes without methods use the default and drop the class
nomethod <- c(x = 1, y = 2)
class(nomethod) <- "nometh"
nomethod[2]
nomethod[["x"]]
nomethod[2] <- 5
class(nomethod)
unclass(nomethod)
# factors
fct <- c(2L, 1L, 2L, 3L)
levels(fct) <- c("lo", "mid", "hi")
class(fct) <- "factor"
unclass(fct[2:3])
unclass(fct[-1, drop = FALSE])
unclass(fct[[4]])
class(fct[1])
levels(fct[1])
# .subset and .subset2 do not dispatch
.subset(rec, 1)
.subset2(rec, "b")
.subset(k, 2:3)
# `@`
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
err(rec@a)
`@.rec` <- function(object, name) c("slot", name)
rec@what
err(k@what)
