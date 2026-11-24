# /dev/full, where every write(2) fails with ENOSPC. A write that stays in
# the C library's buffer succeeds; the flush that fails reports it:
# writeLines' Rconn_printf raises "Error writing to connection", close()
# warns (checkClose, with the errno fclose left) and returns -1. A failed
# flush drops what the buffer held, so later writes start afresh. flush()
# does not look at fflush's result.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) paste(deparse(conditionCall(e)), conditionMessage(e), sep = ": "))
closed <- function(con) withCallingHandlers(close(con), warning = function(w) {
  cat("warning:", deparse(conditionCall(w)), conditionMessage(w), "\n")
  invokeRestart("muffleWarning")
})
con <- file("/dev/full", "w")
writeLines("small", con)
print(closed(con))
# a write that fills the buffer fails inside writeLines
con <- file("/dev/full", "w")
print(msg(writeLines(strrep("x", 5000), con)))
# the buffer was dropped: a small write succeeds; flush() fails silently
writeLines("again", con)
print(flush(con))
print(closed(con))
# strings written one by one: the third fills the buffer
con <- file("/dev/full", "w")
print(msg(writeLines(rep(strrep("y", 2000), 5), con)))
print(closed(con))
# serialize's ascii writes (Rconn_printf), and binary ones (con->write)
con <- file("/dev/full", "w")
serialize(1:3, con, ascii = TRUE)
print(closed(con))
con <- file("/dev/full", "wb")
serialize(1:3, con)
print(closed(con))
con <- file("/dev/full", "wb")
print(msg(serialize((1:1000) / 3, con)))
print(closed(con))
# writeLines opening the connection itself: its close warns
writeLines(c("a", "b"), file("/dev/full"))
cat("done\n")
