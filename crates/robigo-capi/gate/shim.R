# A minimal tinytest/testthat shim: run each test file, count expectations.
args <- commandArgs(TRUE)
pkg <- args[1]; dir <- args[2]
lib <- if (length(args) >= 3) args[3] else "/tmp/rlib"
.libPaths(c(lib, .libPaths()))
library(pkg, character.only = TRUE, lib.loc = lib)
res <- new.env(); res$pass <- 0L; res$fail <- character()
rec <- function(ok, what) {
  if (isTRUE(ok)) res$pass <- res$pass + 1L
  else res$fail <- c(res$fail, paste(res$file, res$desc, what))
  invisible(ok)
}
expect_equal <- function(current, target, ..., info = NULL) rec(isTRUE(all.equal(target, current, ...)), "equal")
expect_equivalent <- function(current, target, ...) rec(isTRUE(all.equal(target, current, check.attributes = FALSE, ...)), "equivalent")
expect_identical <- function(current, target, ...) rec(identical(current, target), "identical")
expect_true <- function(x, ...) rec(isTRUE(x), "true")
expect_false <- function(x, ...) rec(isFALSE(x), "false")
expect_null <- function(x, ...) rec(is.null(x), "null")
expect_error <- function(x, pattern = NULL, ...) rec(inherits(try(x, silent = TRUE), "try-error"), "error")
expect_warning <- function(x, ...) { w <- FALSE; withCallingHandlers(x, warning = function(c) { w <<- TRUE; invokeRestart("muffleWarning") }); rec(w, "warning") }
expect_silent <- function(x, ...) rec(!inherits(try(x, silent = TRUE), "try-error"), "silent")
expect_message <- function(x, ...) { m <- FALSE; withCallingHandlers(x, message = function(c) { m <<- TRUE; invokeRestart("muffleMessage") }); rec(m, "message") }
expect_inherits <- function(current, class, ...) rec(inherits(current, class), "inherits")
using <- function(...) invisible(NULL)
context <- function(...) invisible(NULL)
expect_is <- function(object, class, ...) rec(inherits(object, class), "is")
is_a <- function(class) function(x) inherits(x, class)
equals <- function(expected, ...) function(x) isTRUE(all.equal(expected, x, ...))
is_identical_to <- function(expected) function(x) identical(x, expected)
expect_that <- function(object, condition, ...) rec(isTRUE(condition(object)), "that")
expect_length <- function(x, n, ...) rec(length(x) == n, "length")
ignore <- function(f) f
exit_file <- function(...) stop("exit_file")
at_home <- function() FALSE
test_that <- function(desc, code) {
  res$desc <- desc
  r <- try(eval(substitute(code), new.env(parent = parent.frame())), silent = TRUE)
  if (inherits(r, "try-error")) res$fail <- c(res$fail, paste(res$file, desc, "ERROR:", trimws(conditionMessage(attr(r, "condition")))))
}
expect_mapequal <- function(object, expected, ...) rec(setequal(names(object), names(expected)) && identical(object[sort(names(object))], expected[sort(names(expected))]), "mapequal")
expect_setequal <- function(object, expected, ...) rec(setequal(object, expected), "setequal")
skip_on_cran <- function() invisible(NULL)
skip_if_not <- function(...) invisible(NULL)
skip <- function(...) stop("skip")
# testthat evaluates test files in a child of the package namespace;
# tinytest in a child of the global environment
parent <- if (basename(normalizePath(dir)) == "testthat") new.env(parent = asNamespace(pkg)) else globalenv()
for (h in list.files(dir, pattern = "^helper.*\\.R$", full.names = TRUE)) sys.source(h, envir = parent)
for (f in sort(list.files(dir, pattern = "^test.*\\.R$", full.names = TRUE))) {
  res$file <- basename(f)
  r <- try(sys.source(f, envir = new.env(parent = parent)), silent = TRUE)
  if (inherits(r, "try-error")) res$fail <- c(res$fail, paste(basename(f), "ABORTED:", trimws(conditionMessage(attr(r, "condition")))))
}
cat(pkg, ": ", res$pass, " expectations passed, ", length(res$fail), " failed\n", sep = "")
if (length(res$fail)) writeLines(paste(" -", res$fail))
