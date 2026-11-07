# needs: testthat
# testthat running a user test file end to end: a passing, a failing
# and an erroring test, counted the same. (Edition and snapshot
# state across tests is the canary's claim, tests/packages/canary.)
suppressPackageStartupMessages(library(testthat))
f <- tempfile(fileext = ".R")
writeLines(c(
  "test_that('passes', { expect_equal(1 + 1, 2); expect_true(TRUE) })",
  "test_that('fails', { expect_equal(1, 2) })",
  "test_that('errors', { stop('inner boom') })",
  "test_that('skips', { skip('not today') })"
), f)
res <- test_file(f, reporter = "silent")
df <- as.data.frame(res)
print(df[c("test", "nb", "failed", "error", "skipped")])
