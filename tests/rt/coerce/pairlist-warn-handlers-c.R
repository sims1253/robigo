# fill_pairlist warning-stop cases under LC_ALL=C (H's five extra cases):
# (1) a calling handler that allocates heavily (paste0 of 1e4) between cells,
# (2) a calling handler that errors,
# (3) a calling handler that calls as.pairlist again (re-entrant),
# (4) two untranslatable names with a collection forced between them,
# (5) a bytes-encoded name after an untranslatable name.
u1 <- intToUtf8(c(99L, 97L, 102L, 233L))
u2 <- intToUtf8(c(110L, 97L, 239L, 118L, 101L))
b <- "\x80"
Encoding(b) <- "bytes"

# 1. Handler allocates heavily (paste0 of 1e4) + gc() between cells
cat("-- heavy handler (list)\n")
l <- list(10L, 20L, 30L)
names(l) <- c(u1, "ok", u2)
did_big <- FALSE
res1 <- withCallingHandlers(
  as.pairlist(l),
  warning = function(w) {
    cat("warn:", conditionMessage(w), "\n")
    if (!did_big) {
      did_big <<- TRUE
      big <- paste0("h", seq_len(10000))
      cat("alloc len:", length(big), "\n")
    }
    invisible(gc())
    invokeRestart("muffleWarning")
  }
)
print(res1)

cat("-- heavy handler (atomic)\n")
a <- c(1L, 2L, 3L)
names(a) <- c(u1, "ok", u2)
res2 <- withCallingHandlers(
  as.pairlist(a),
  warning = function(w) {
    cat("warn:", conditionMessage(w), "\n")
    big <- paste0("h", seq_len(200))
    cat("alloc len:", length(big), "\n")
    invisible(gc())
    invokeRestart("muffleWarning")
  }
)
print(res2)

# 2. Handler errors mid-fill (unwind-safety)
cat("-- error handler\n")
err <- tryCatch(
  withCallingHandlers(
    as.pairlist(l),
    warning = function(w) {
      cat("warn before stop:", conditionMessage(w), "\n")
      stop("handler abort")
    }
  ),
  error = function(e) conditionMessage(e)
)
cat("caught:", err, "\n")

# 3. Handler calls as.pairlist again (re-entrant, both list and atomic)
cat("-- reentrant handler\n")
inner_seen <- list()
res3 <- withCallingHandlers(
  as.pairlist(l),
  warning = function(w) {
    cat("outer warn:", conditionMessage(w), "\n")
    inner_seen[[length(inner_seen) + 1L]] <<- as.pairlist(c(p = 1L, q = 2L))
    inner_seen[[length(inner_seen) + 1L]] <<- as.pairlist(list(r = 3L, s = 4L))
    invokeRestart("muffleWarning")
  }
)
print(res3)
print(inner_seen)

# 4. Two untranslatable names with explicit gc() forced between them
cat("-- two untranslatable with gc between\n")
l2 <- list(1L, 2L)
names(l2) <- c(u1, u2)
res4 <- withCallingHandlers(
  as.pairlist(l2),
  warning = function(w) {
    cat("warn:", conditionMessage(w), "\n")
    invisible(gc())
    invokeRestart("muffleWarning")
  }
)
print(names(res4))

# 5. Untranslatable name followed by bytes-encoded name (both list and atomic)
cat("-- utf8 then bytes (list)\n")
lb <- list(1L, 2L, 3L)
names(lb) <- c(u1, b, u2)
err_lb <- tryCatch(
  withCallingHandlers(
    as.pairlist(lb),
    warning = function(w) {
      cat("warn:", conditionMessage(w), "\n")
      invokeRestart("muffleWarning")
    }
  ),
  error = function(e) conditionMessage(e)
)
cat("err:", err_lb, "\n")

cat("-- utf8 then bytes (atomic)\n")
ab <- c(1L, 2L, 3L)
names(ab) <- c(u1, b, u2)
err_ab <- tryCatch(
  withCallingHandlers(
    as.pairlist(ab),
    warning = function(w) {
      cat("warn:", conditionMessage(w), "\n")
      invokeRestart("muffleWarning")
    }
  ),
  error = function(e) conditionMessage(e)
)
cat("err:", err_ab, "\n")
