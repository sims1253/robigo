# Replacement methods whose `value` comes after `...`: the dispatch passes
# the value tagged `value =`, so it matches by name (AST and compiled paths)
t <- function(expr) tryCatch(expr, error = function(e) cat("error:", conditionMessage(e), "\n"))
show <- function(expr) t(print(expr))
x <- structure(list(a = 1), class = "foo")
`$<-.foo` <- function(x, name, ..., value) { cat("$<- name", name, "value", value, "dots", length(list(...)), "\n"); x }
`[<-.foo` <- function(x, ..., value) { cat("[<- value", value, "dots", ...length(), "\n"); x }
`[[<-.foo` <- function(x, ..., value) { cat("[[<- value", value, "dots", ...length(), "\n"); x }
`names<-.foo` <- function(x, ..., value) { cat("names<- value", value, "\n"); x }
`dim<-.foo` <- function(x, ..., value) { cat("dim<- value", value, "\n"); x }
`dimnames<-.foo` <- function(x, ..., value) { cat("dimnames<- value", unlist(value), "\n"); x }
`levels<-.foo` <- function(x, ..., value) { cat("levels<- value", value, "\n"); x }
`length<-.foo` <- function(x, ..., value) { cat("length<- value", value, "\n"); x }
# at top level (the AST path)
t(x$b <- 2)
t(x[1] <- 3)
t(x[1, 2] <- 4)
t(x[[1]] <- 5)
t(x[["a", exact = TRUE]] <- 6)
t(names(x) <- "n")
t(dim(x) <- 1L)
t(dimnames(x) <- list("d"))
t(levels(x) <- "l")
t(length(x) <- 7)
# called directly, with value named and not
show(`$<-`(x, "b", value = 8))
show(`[<-`(x, 1, value = 9))
show(`[[<-`(x, 1, value = 10))
show(`names<-`(x, value = "m"))
# in a function: compiled (the JIT) where GNU R compiles
f <- function(x) {
    t(x$b <- 11)
    t(x[1] <- 12)
    t(x[[1]] <- 13)
    t(names(x) <- "o")
    t(dim(x) <- 1L)
    t(dimnames(x) <- list("e"))
    t(levels(x) <- "k")
    t(length(x) <- 14)
    x
}
invisible(f(x)); invisible(f(x)); invisible(f(x))
