# Writing to stderr() follows sink(type = "message"): R's stderr connection
# prints through REvprintf, which writes to R_ErrorCon (connections.c:
# stderr_vfprintf). capture.output(type = "message") relies on it.
zz <- textConnection("msgs", "w")
sink(zz, type = "message")
message("from message()")
cat("from cat\n", file = stderr())
writeLines("from writeLines", stderr())
warning("a warning")
sink(type = "message")
close(zz)
print(msgs)
print(utils::capture.output(message("captured"), type = "message"))
print(utils::capture.output(cat("a", "b\n", file = stderr()), type = "message"))
print(utils::capture.output(suppressMessages(message("hidden")), type = "message"))
# cat() to stdout() goes where the output sink sends it
out <- utils::capture.output(cat("to stdout()\n", file = stdout()), cat("sunk\n"))
print(out)
# a file connection is flushed after each message
f <- tempfile()
con <- file(f, "w")
sink(con, type = "message")
message("first")
n <- length(readLines(f))
sink(type = "message")
close(con)
print(n)
print(readLines(f))
