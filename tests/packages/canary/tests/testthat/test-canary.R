# Each test's name is its expected outcome: scripts/package-suite.py checks
# the reported outcome of every one against CANARY. A mismatch means the
# results of this run cannot be trusted, whatever the packages scored.
local_edition(3)

test_that("pass", expect_equal(1 + 1, 2))
test_that("fail", expect_equal(1 + 1, 3))
test_that("error", stop("deliberate"))
test_that("skip", skip("deliberate"))
test_that("warn", {
  warning("deliberate")
  expect_true(TRUE)
})
test_that("pass: an unexported function, through the cloned namespace", {
  expect_equal(hidden(), "hidden")
})
test_that("pass: the edition survives an earlier test", {
  expect_equal(edition_get(), 3L)
})
test_that("pass: snapshot match", expect_snapshot(cat("matches\n")))
test_that("fail: snapshot mismatch", expect_snapshot(cat("differs\n")))
