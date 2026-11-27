# errors.c do_stop / do_warning translateChar under LC_ALL=C.
# This copy runs under LC_ALL=C (its .locale sidecar).
x <- intToUtf8(c(99L, 97L, 102L, 233L))
b <- "\x80"
Encoding(b) <- "bytes"

probe <- function(lab, expr) {
  cat("--", lab, "\n")
  r <- withCallingHandlers(
    tryCatch(expr, error = function(e) { cat("error:", conditionMessage(e), "\n"); invisible(NULL) }),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }
  )
  if (!is.null(r)) print(r)
}

probe("stop utf8", {
  m <- sprintf("object %s not found", sQuote(x))
  e <- tryCatch(stop(m), error = function(e) e)
  list(Encoding(conditionMessage(e)), conditionMessage(e))
})
probe("stop bytes", { stop(b) })
probe("stop NA", { e <- tryCatch(stop(NA_character_), error = function(e) e); conditionMessage(e) })

probe("warning utf8", {
  w <- NULL
  r <- withCallingHandlers(warning(x), warning = function(c) { w <<- c; invokeRestart("muffleWarning") })
  list(Encoding(conditionMessage(w)), conditionMessage(w), Encoding(r))
})
probe("warning bytes", { warning(b) })
probe("warning NA", {
  w <- NULL
  withCallingHandlers(warning(NA_character_), warning = function(c) { w <<- c; invokeRestart("muffleWarning") })
  conditionMessage(w)
})
