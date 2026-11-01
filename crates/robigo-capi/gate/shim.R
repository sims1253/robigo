# A minimal tinytest/testthat shim: run each test file, count expectations.
#
#   R --vanilla --quiet -f shim.R --args PKG TESTS_DIR [LIB]
#
# Prints one line `PKG VERSION: N expectations passed, M failed` and, when
# something failed, one ` - FILE DESC WHAT` line per failure. Test files run
# with the working directory set to TESTS_DIR, as tinytest's run_test_dir()
# and testthat's test_dir() do (packages hash a file of their own).
#
# It must behave the same under GNU R and robigo, so it is plain base R: no
# testthat, no tinytest. The API below is what the packages' tests call; a
# function added here must be exact enough that a pass means the same thing
# in both implementations, otherwise the comparison hides a real failure.
# Snapshots cannot be compared without testthat's snapshot files: they count
# as one passing expectation on both sides (the code under test still runs,
# so an error there is a failure).
args <- commandArgs(TRUE)
pkg <- args[1]; dir <- args[2]
lib <- if (length(args) >= 3) args[3] else "/tmp/rlib"
.libPaths(c(lib, .libPaths()))
library(pkg, character.only = TRUE, lib.loc = lib)
res <- new.env(); res$pass <- 0L; res$fail <- character(); res$file <- ""; res$desc <- ""
rec <- function(ok, what) {
  if (isTRUE(ok)) res$pass <- res$pass + 1L
  else res$fail <- c(res$fail, paste(res$file, res$desc, what))
  invisible(ok)
}
err <- function(x) inherits(try(x, silent = TRUE), "try-error")
msg_of <- function(x) tryCatch({ x; "" }, error = function(e) trimws(conditionMessage(e)),
                               warning = function(w) trimws(conditionMessage(w)),
                               message = function(m) trimws(conditionMessage(m)))
caught <- function(expr, class) {
  got <- NULL
  withCallingHandlers(try(expr, silent = TRUE),
    warning = function(w) { if (is.null(got)) got <<- list("warning", trimws(conditionMessage(w))); invokeRestart("muffleWarning") },
    message = function(m) { if (is.null(got)) got <<- list("message", trimws(conditionMessage(m))); invokeRestart("muffleMessage") },
    error = function(e) { if (is.null(got)) got <<- list("error", trimws(conditionMessage(e))) })
  !is.null(got) && (is.null(class) || got[[1]] == class)
}
rec_cond <- function(expr, class, pattern, what) {
  seen <- NULL
  # warnings and messages must be handled while the condition is being
  # signalled (that is where their muffle restart lives), errors after
  # unwinding to this frame, as testthat does
  tryCatch(withCallingHandlers(expr,
      warning = function(w) {
        if (is.null(seen)) seen <<- list("warning", trimws(conditionMessage(w)))
        if (!is.null(findRestart("muffleWarning"))) invokeRestart("muffleWarning")
      },
      message = function(m) {
        if (is.null(seen)) seen <<- list("message", trimws(conditionMessage(m)))
        if (!is.null(findRestart("muffleMessage"))) invokeRestart("muffleMessage")
      }),
    error = function(e) if (is.null(seen)) seen <<- list("error", trimws(conditionMessage(e))))
  ok <- !is.null(seen) && (is.null(class) || seen[[1]] == class) &&
    (is.null(pattern) || isTRUE(grepl(pattern, seen[[2]])))
  invisible(rec(ok, what))
}
out_of <- function(expr) paste(capture.output(expr), collapse = "\n")
# ---- expectations -----------------------------------------------------------
expect_equal <- function(current, target, ..., info = NULL, label = NULL, expected.label = NULL) rec(isTRUE(all.equal(target, current, ...)), "equal")
expect_equivalent <- function(current, target, ...) rec(isTRUE(all.equal(target, current, check.attributes = FALSE, ...)), "equivalent")
expect_identical <- function(current, target, ..., info = NULL, label = NULL) rec(identical(current, target), "identical")
expect_true <- function(x, ..., info = NULL, label = NULL) rec(isTRUE(x), "true")
expect_false <- function(x, ..., info = NULL, label = NULL) rec(isFALSE(x), "false")
expect_null <- function(x, ...) rec(is.null(x), "null")
expect_length <- function(x, n, ...) rec(length(x) == n, "length")
expect_named <- function(x, nm = NULL, ignore.order = FALSE, ignore.case = FALSE, ...) {
  got <- names(x)
  if (is.null(nm)) return(rec(!is.null(got), "named"))
  if (ignore.order) got <- sort(got)
  if (ignore.case) got <- tolower(got)
  rec(identical(got, if (ignore.case) tolower(nm) else if (ignore.order) sort(nm) else nm), "named")
}
expect_type <- function(x, type, ...) rec(typeof(x) == type, paste0("type ", type))
expect_s3_class <- function(x, class, exact = FALSE, ...) {
  ok <- if (exact) identical(class(x), class) else inherits(x, class)
  rec(ok, paste0("s3 class ", paste(class, collapse = "/")))
}
expect_s4_class <- function(x, class, ...) rec(is(x, class), paste0("s4 class ", class))
expect_inherits <- function(x, class, ...) rec(inherits(x, class), "inherits")
expect_is <- function(x, class, ...) rec(inherits(x, class), "is")
expect_match <- function(object, regexp, perl = FALSE, fixed = FALSE, all = TRUE, ...) {
  ok <- if (all) all(grepl(regexp, object, perl = perl, fixed = fixed)) else any(grepl(regexp, object, perl = perl, fixed = fixed))
  rec(isTRUE(ok) && length(object) > 0, "match")
}
expect_output <- function(object, regexp = NULL, ...) {
  o <- out_of(object)
  rec(is.null(regexp) || grepl(regexp, o), "output")
}
expect_silent <- function(object, ...) {
  o <- NULL
  r <- withCallingHandlers(tryCatch(object, error = function(e) { o <<- "error"; NULL }),
        warning = function(w) { o <<- "warning"; invokeRestart("muffleWarning") },
        message = function(m) { o <<- "message"; invokeRestart("muffleMessage") })
  rec(is.null(o), "silent")
}
expect_error <- function(object, regexp = NULL, class = NULL, ..., pattern = NULL) rec_cond(object, "error", if (missing(regexp)) pattern else regexp, "error")
expect_warning <- function(object, regexp = NULL, class = NULL, ..., pattern = NULL) rec_cond(object, "warning", if (missing(regexp)) pattern else regexp, "warning")
expect_message <- function(object, regexp = NULL, class = NULL, ..., pattern = NULL) rec_cond(object, "message", if (missing(regexp)) pattern else regexp, "message")
expect_condition <- function(object, regexp = NULL, class = NULL, ..., pattern = NULL) rec_cond(object, class, if (missing(regexp)) pattern else regexp, "condition")
expect_no_error <- function(object, ..., message = NULL) rec(!err(object), "no error")
expect_no_warning <- function(object, ..., message = NULL) rec(!caught(object, "warning"), "no warning")
expect_no_message <- function(object, ..., message = NULL) rec(!caught(object, "message"), "no message")
expect_visible <- function(expr, ...) rec(!withVisible(expr)$visible, "visible")
expect_invisible <- function(expr, ...) rec(!withVisible(expr)$visible, "invisible")
expect_snapshot <- function(x, ..., cran = FALSE) { force(x); rec(TRUE, "snapshot") }
expect_snapshot_output <- function(x, ...) { force(x); rec(TRUE, "snapshot") }
expect_snapshot_value <- function(x, ..., style = "json2", cran = FALSE) { force(x); rec(TRUE, "snapshot") }
expect_snapshot_error <- function(x, ..., cran = FALSE) { force(x); rec(TRUE, "snapshot") }
expect_snapshot_warning <- function(x, ..., cran = FALSE) { force(x); rec(TRUE, "snapshot") }
expect_shape <- function(object, shape, ...) rec(identical(dim(object), as.integer(shape)), "shape")
expect_gt <- function(object, expected, ...) rec(all(object > expected), "gt")
expect_gte <- function(object, expected, ...) rec(all(object >= expected), "gte")
expect_lt <- function(object, expected, ...) rec(all(object < expected), "lt")
expect_lte <- function(object, expected, ...) rec(all(object <= expected), "lte")
expect_mapequal <- function(object, expected, ...) rec(setequal(names(object), names(expected)) && identical(object[sort(names(object))], expected[sort(names(expected))]), "mapequal")
expect_setequal <- function(object, expected, ...) rec(setequal(object, expected), "setequal")
expect_that <- function(object, condition, ...) rec(isTRUE(condition(object)), "that")
expect_waldo_equal <- function(x, y, ...) rec(identical(x, y), "waldo equal")
expect_r6_class <- function(object, class, ...) rec(inherits(object, class), paste0("r6 class ", class))
# tinytest's condition recorders
expect_true_or_false <- expect_true
using <- function(...) invisible(NULL)
context <- function(...) invisible(NULL)
equals <- function(expected, ...) function(x) isTRUE(all.equal(expected, x, ...))
is_a <- function(class) function(x) inherits(x, class)
is_identical_to <- function(expected) function(x) identical(x, expected)
ignore <- function(f) f
# ---- skips ------------------------------------------------------------------
exit_file <- function(msg = "") {
  stop(structure(class = c("tinytest_exit_file", "condition"), list(message = msg, call = NULL)))
}
skip <- function(...) exit_file(...)
skip_if <- function(condition, msg = NULL) if (isTRUE(condition)) exit_file(msg %||% "")
skip_if_not <- function(condition, msg = NULL) if (!isTRUE(condition)) exit_file(msg %||% "")
skip_if_not_installed <- function(pkg, minimum_version = NULL) {
  if (!requireNamespace(pkg, quietly = TRUE)) exit_file(paste("package", pkg, "not installed"))
  if (!is.null(minimum_version) && packageVersion(pkg) < minimum_version) exit_file(paste("package", pkg, "too old"))
}
skip_if_installed <- function(pkg) if (requireNamespace(pkg, quietly = TRUE)) exit_file(paste("package", pkg, "installed"))
skip_on_cran <- function() invisible(NULL)
skip_on_os <- function(os, ...) {
  mine <- if (.Platform$OS.type == "windows") "windows" else Sys.info()[["sysname"]]
  mine <- tolower(mine)
  if (mine %in% tolower(os)) exit_file(paste("on", mine))
}
skip_on_os_arch <- function(arch, ...) invisible(NULL)
skip_on_ci <- function() invisible(NULL)
skip_on_covr <- function() invisible(NULL)
skip_if_offline <- function(...) invisible(NULL)
skip_if_not_rstudio <- function() exit_file("not RStudio")
skip_on_bioc <- function() invisible(NULL)
skip_if_translated <- function() invisible(NULL)
skip_if_fast <- function() invisible(NULL)
`%||%` <- function(x, y) if (is.null(x)) y else x
at_home <- function() TRUE
# ---- test_that / describe-it ------------------------------------------------
rec_error <- function(e) {
  res$fail <- c(res$fail, paste(res$file, res$desc, "ERROR:", trimws(conditionMessage(e))))
}
test_that <- function(desc, code) {
  res$desc <- desc
  withCallingHandlers(tryCatch(eval(substitute(code), new.env(parent = parent.frame())),
                               error = rec_error,
                               tinytest_exit_file = function(e) NULL),
                      warning = function(w) invokeRestart("muffleWarning"))
}
describe <- function(desc, code) {
  old <- res$desc; res$desc <- desc
  try(eval(substitute(code), new.env(parent = parent.frame())), silent = TRUE)
  res$desc <- old
}
it <- function(desc, code) test_that(desc, code)
# testthat's local_* helpers only change state for the duration of a test;
# with no snapshot files and no reporter, they can be no-ops.
local_edition <- function(...) invisible(NULL)
local_reproducible_output <- function(...) invisible(NULL)
local_options <- function(...) invisible(NULL)
local_envvar <- function(...) invisible(NULL)
local_tempdir <- function(pattern = "file") tempfile(pattern)
local_tempfile <- function(...) tempfile()
local_dir <- function(...) tempfile()
local_mocked_bindings <- function(...) invisible(NULL)
with_mocked_bindings <- function(code, ...) force(code)
local_language <- function(...) invisible(NULL)
local_theme <- function(...) invisible(NULL)
local_test_context <- function(...) invisible(NULL)
test_path <- function(...) file.path(dir, ...)
# testthat evaluates test files in a child of the package namespace;
# tinytest in a child of the global environment
parent <- if (basename(normalizePath(dir)) == "testthat") new.env(parent = asNamespace(pkg)) else globalenv()
here <- getwd()
setwd(dir)
for (h in list.files(".", pattern = "^helper.*\\.R$", full.names = TRUE)) sys.source(h, envir = parent)
for (f in sort(list.files(".", pattern = "^test.*\\.R$", full.names = TRUE))) {
  res$file <- basename(f); res$desc <- ""
  msg <- tryCatch({
    sys.source(f, envir = new.env(parent = parent))
    NA_character_
  }, tinytest_exit_file = function(e) NA_character_,
     error = function(e) trimws(conditionMessage(e)))
  if (!is.na(msg)) res$fail <- c(res$fail, paste(basename(f), "ABORTED:", msg))
}
setwd(here)
cat(pkg, " ", as.character(packageVersion(pkg)), ": ", res$pass,
    " expectations passed, ", length(res$fail), " failed\n", sep = "")
if (length(res$fail)) writeLines(paste(" -", res$fail))
