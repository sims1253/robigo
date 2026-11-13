# cat(file = stderr()) writes to the error stream.
cat("to stdout\n")
cat("to stderr\n", file = stderr())
cat("again stdout\n", file = stdout())
