# A file connection writes through the C library's buffer (glibc's FILE
# under connections.c's file_write and file_vfprintf): another reader sees
# the bytes after flush(), after close(), or once the buffer fills (the
# file's st_blksize, 4096 bytes here). A connection open for reading and
# writing switches between the two with fseeko, which writes what is
# buffered first.
f <- tempfile()
con <- file(f, "w")
writeLines(c("one", "two"), con)
print(file.size(f))
print(readLines(f))
flush(con)
print(file.size(f))
print(readLines(f))
writeLines(strrep("a", 5000), con)
print(file.size(f))
writeLines("tail", con)
print(file.size(f))
print(close(con))
print(file.size(f))
x <- readLines(f)
print(nchar(x))
# "w+": written, then read back on the same connection
con <- file(f, "w+")
writeLines(c("x", "y"), con)
print(file.size(f))
print(readLines(con))
writeLines("z", con)
print(close(con))
print(readLines(f))
# "a": at the end, when the buffer is written
con <- file(f, "a")
writeLines("appended", con)
print(readLines(f))
print(close(con))
print(readLines(f))
# flush(): nothing for a connection open only for reading; the console
# for stdout(); the connection checks
con <- file(f, "r")
print(flush(con))
print(readLines(con, 1))
close(con)
cat("before")
flush(stdout())
cat(" after\n")
print(tryCatch(flush(1), error = conditionMessage))
print(tryCatch(flush(structure(99L, class = "connection")), error = conditionMessage))
# serialize: ascii writes are Rconn_printf's, binary ones con->write's
con <- file(f, "w")
serialize(1:3, con, ascii = TRUE)
print(file.size(f))
close(con)
print(readLines(f, 2))
con <- file(f, "wb")
serialize(1:3, con)
print(file.size(f))
close(con)
print(file.size(f))
invisible(file.remove(f))
