# readChar (connections.c's do_readchar, readFixedString, rawFixedString):
# characters or bytes of a file, an open connection or a raw vector, the
# NUL warning once per call, invalid UTF-8, a text connection's warning,
# and the argument errors.
f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-readchar-", Sys.getpid())
msg <- function(expr) sub(f, "F", tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e),
                                           warning = function(w) paste("warning:", conditionMessage(w))), fixed = TRUE)
d <- if (dir.exists("data")) "data" else "tests/rt/connections/data"
writeLines(c("abcdef", "ghij"), f)
print(readChar(f, 3))
print(readChar(f, c(1, 2, 3, 100)))
print(readChar(f, c(2, 0, 2)))
print(readChar(f, integer(0)))
print(readChar(f, 0))
print(readChar(f, 100, useBytes = TRUE))
print(readChar(f, 2.7))
print(msg(readChar(f, NA)))
print(msg(readChar(f, -1)))
print(msg(readChar(f, c(1, NA))))
print(msg(readChar(f, 3, useBytes = NA)))
con <- file(f, "rb")
print(readChar(con, 2)); print(readChar(con, 3)); print(readChar(con, 100)); print(readChar(con, 5))
close(con)
con <- file(f, "r")
print(msg(readChar(con, 2)))
print(suppressWarnings(readChar(con, 2)))
close(con)
con <- file(f, "w")
print(msg(readChar(con, 2)))
close(con)
# UTF-8: characters, or bytes with useBytes
u <- file.path(d, "utf8.txt")
print(readChar(u, c(1, 3, 5)))
print(readChar(u, c(1, 3, 5), useBytes = TRUE))
# NULs: the string cut at the first, the warning once
n <- file.path(d, "nul.txt")
print(msg(readChar(n, 100)))
print(suppressWarnings(readChar(n, c(2, 2, 2, 2))))
r <- withCallingHandlers(readChar(n, c(2, 2, 2, 2, 2), useBytes = TRUE),
                         warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
print(r)
print(msg(readChar(file.path(d, "nul-c3.bin"), 2)))
print(msg(suppressWarnings(readChar(file.path(d, "nul-c3.bin"), 2))))
# the connection's own read: what a text reader buffered is not read again
writeLines(c("a", "bc"), f)
con <- file(f, "r")
print(readLines(con, 1))
print(suppressWarnings(readChar(con, 1)))
close(con)
# invalid UTF-8
writeLines("a\xe2\x82", f, useBytes = TRUE)
print(msg(readChar(f, 3)))
print(readChar(f, 3, useBytes = TRUE))
# raw vectors
r <- as.raw(c(0x61, 0x62, 0xc3, 0xa9, 0x63))
print(readChar(r, c(1, 2, 5)))
print(readChar(r, c(1, 2, 5), useBytes = TRUE))
print(readChar(r, c(2, 2, 2, 2)))
print(readChar(raw(0), 3))
print(msg(readChar(as.raw(c(0x61, 0xe2, 0x82)), 2)))
print(msg(readChar(as.raw(c(0x61, 0, 0x62)), 3, TRUE)))
print(msg(readChar(as.raw(c(0x61, 0x62)), NA)))
# a connection that is not one
print(msg(readChar(99L, 1)))
invisible(file.remove(f))
