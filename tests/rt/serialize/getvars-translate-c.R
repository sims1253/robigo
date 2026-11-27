# serialize.c do_getVarsFromFrame installTrChar under LC_ALL=C (and
# R-level save(list = x) which exercises stop() translation too).
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

e <- new.env(parent = emptyenv())
probe("getVarsFromFrame missing utf8", { .Internal(getVarsFromFrame(x, e, FALSE)) })
probe("getVarsFromFrame bytes", { .Internal(getVarsFromFrame(b, e, FALSE)) })
probe("getVarsFromFrame empty", { .Internal(getVarsFromFrame("", e, FALSE)) })

assign("caf<U+00E9>", 42L, envir = e)
probe("getVarsFromFrame found utf8", { .Internal(getVarsFromFrame(x, e, FALSE)) })

probe("save missing utf8", { save(list = x, file = tempfile(), envir = new.env(parent = emptyenv())) })
