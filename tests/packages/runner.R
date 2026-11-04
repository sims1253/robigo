# Run one test file of an installed package with its real test framework and
# stream what happens to PKG_SUITE_OUT, one JSON line per event, flushed as it
# happens: a crash, a kill or a timeout keeps everything before it. Driven by
# scripts/package-suite.py, one process per file, the same under GNU R and
# robigo.
#
# Base R only past the framework itself. Reporting must not depend on
# jsonlite or on as.data.frame.testthat_results, which are part of what is
# being tested. Everything lives in local(): a global variable of the runner
# would be visible to the tests (tidyselect has one that checks an unbound `a`).
local({
  pkg <- Sys.getenv("PKG_SUITE_PKG")
  path <- Sys.getenv("PKG_SUITE_FILE")
  fw <- Sys.getenv("PKG_SUITE_FRAMEWORK")
  con <- file(Sys.getenv("PKG_SUITE_OUT"), open = "a", encoding = "UTF-8")

  js <- function(s) {
    s <- tryCatch({
      s <- substr(enc2utf8(paste(s, collapse = "\n")), 1L, 500L)
      for (p in list(c("\\", "\\\\"), c("\"", "\\\""), c("\n", "\\n"), c("\t", "\\t"), c("\r", "\\r")))
        s <- gsub(p[1L], p[2L], s, fixed = TRUE)
      gsub("[[:cntrl:]]", " ", s)
    }, error = function(e) "<unprintable>")
    paste0("\"", s, "\"")
  }
  emit <- function(...) {
    writeLines(paste0(...), con)
    flush(con)
  }
  kind <- function(r) {
    for (k in c("success", "failure", "error", "skip", "warning"))
      if (inherits(r, paste0("expectation_", k))) return(k)
    "error"
  }
  message_of <- function(r) tryCatch(conditionMessage(r), error = function(e) "<no message>")

  run_testthat <- function() {
    Suite <- R6::R6Class("SuiteReporter", inherit = testthat::Reporter, public = list(
      start_test = function(context, test) emit("{\"e\":\"start\",\"test\":", js(test), "}"),
      add_result = function(context, test, result) {
        k <- kind(result)
        emit("{\"e\":\"result\",\"kind\":\"", k, "\"",
             if (k != "success") paste0(",\"msg\":", js(message_of(result))), "}")
      },
      end_test = function(context, test) emit("{\"e\":\"end\"}")
    ))
    testthat::test_file(path, reporter = Suite$new(), package = pkg, load_package = "installed")
  }

  # tinytest reports a file at its end, one result per expectation: the
  # expectation is its test block, named by its line.
  run_tinytest <- function() {
    res <- tinytest::run_test_file(path, at_home = FALSE, verbose = 0, color = FALSE)
    for (r in res) {
      a <- attributes(r)
      emit("{\"e\":\"start\",\"test\":", js(paste0("line ", a$fst, ": ", deparse(a$call)[1L])), "}")
      emit("{\"e\":\"result\",\"kind\":\"", if (isTRUE(as.logical(r))) "success" else "failure", "\"",
           if (!isTRUE(as.logical(r))) paste0(",\"msg\":", js(a$diff)), "}")
      emit("{\"e\":\"end\"}")
    }
  }

  emit("{\"e\":\"begin\"}")
  status <- tryCatch({
    suppressPackageStartupMessages({
      library(fw, character.only = TRUE)
      library(pkg, character.only = TRUE)
    })
    if (fw == "testthat") run_testthat() else run_tinytest()
    0L
  }, error = function(e) {
    emit("{\"e\":\"abort\",\"msg\":", js(message_of(e)), "}")
    3L
  })
  if (status == 0L) emit("{\"e\":\"done\"}")
  close(con)
  quit("no", status = status, runLast = FALSE)
})
