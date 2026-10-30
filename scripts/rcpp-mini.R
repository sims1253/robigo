# Compare Rcpp's test files with the same expectation shim in GNU R and robigo.
# Usage: robigo scripts/rcpp-mini.R LIB TEST_DIRECTORY OUTPUT [FILE_REGEX]
# Run from TEST_DIRECTORY; install Rcpp with --install-tests and tinytest first.
# This shim checks assertion results; warning/error patterns are not checked.
# RCPP_TEST_R_BIN can select the bin directory of a compatible R CMD toolchain.
args <- commandArgs(TRUE); lib <- args[1]; dir <- args[2]; out <- args[3]
.libPaths(lib)
Sys.setenv(RunAllRcppTests = "yes")
suppressMessages(library(Rcpp))
library(datasets)
compiler_bin <- Sys.getenv("RCPP_TEST_R_BIN")
if (nzchar(compiler_bin)) {
  original_home <- R.home
  compiler_home <- function(component = "home") if (component == "bin") compiler_bin else original_home(component)
  unlockBinding("R.home", baseenv())
  assign("R.home", compiler_home, envir = baseenv())
  lockBinding("R.home", baseenv())
}
res <- character()
cur <- ""
rec <- function(ok) { res[[length(res) + 1L]] <<- paste(cur, length(res) + 1L, if (isTRUE(ok)) "PASS" else "FAIL"); invisible(isTRUE(ok)) }
expect_equal <- function(current, target, tolerance = sqrt(.Machine$double.eps), ...) rec(isTRUE(all.equal(target, current, tolerance = tolerance, ...)))
expect_equivalent <- function(current, target, ...) rec(isTRUE(all.equal(target, current, check.attributes = FALSE, ...)))
expect_identical <- function(current, target, ...) rec(identical(current, target))
expect_true <- function(current, ...) rec(isTRUE(current))
expect_false <- function(current, ...) rec(isFALSE(current))
expect_null <- function(current, ...) rec(is.null(current))
expect_inherits <- function(current, class, ...) rec(inherits(current, class))
expect_error <- function(current, pattern = ".*", ...) rec(inherits(try(current, silent = TRUE), "try-error"))
expect_warning <- function(current, ...) { w <- FALSE; withCallingHandlers(try(current, silent = TRUE), warning = function(c) { w <<- TRUE; invokeRestart("muffleWarning") }); rec(w) }
expect_message <- function(current, ...) { m <- FALSE; withCallingHandlers(try(current, silent = TRUE), message = function(c) { m <<- TRUE; invokeRestart("muffleMessage") }); rec(m) }
expect_silent <- function(current, ...) rec(!inherits(try(current, silent = TRUE), "try-error"))
expect_stdout <- function(current, ...) { o <- capture.output(current); rec(length(o) > 0) }
exit_file <- function(msg = "") stop(structure(class = c("exitFile", "condition"), list(message = msg, call = NULL)))
exit_if_not <- function(...) if (!all(...)) exit_file("exit_if_not")
using <- function(...) invisible(NULL)
if (!file.exists(out)) file.create(out)
for (f in sort(list.files(dir, pattern = if (length(args) > 3) args[4] else "^test_.*\\.R$", full.names = TRUE))) {
  cur <- basename(f)
  cat("FILE", cur, "\n", file = stderr())
  res <- character()
  e <- new.env(parent = globalenv())
  r <- tryCatch({ sys.source(f, envir = e, keep.source = FALSE); "ok" },
                exitFile = function(c) "exit",
                error = function(c) paste("ERROR:", conditionMessage(c)))
  if (r != "ok") cat(r, "\n", file = stderr())
  if (r != "ok") res[[length(res) + 1L]] <- paste(cur, "end", substr(gsub("\n", " ", r), 1, 150))
  cat(res, file = out, sep = "\n", append = TRUE)
}
