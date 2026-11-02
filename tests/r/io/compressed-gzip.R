# R: connections.c:do_file — a gzip file opened for reading in text mode
# becomes a gz connection whose class attribute stays "file".
f <- tempfile()
con <- gzfile(f, "w")
writeLines(as.character(1:3), con)
close(con)

x <- file(f)
cat(class(x), "\n")
cat(readLines(x), sep = "|")
cat("\n")
close(x)

# The connection is opened lazily, as R's checked_open does.
y <- file(f, "rt")
cat(readLines(y), sep = "|")
cat("\n")
close(y)

# raw = TRUE never decompresses; a binary mode skips the detection on its
# own, because do_file only looks at open "" / "r" / "rt".
z <- file(f, "rb", raw = TRUE)
cat(readBin(z, "raw", 2), "\n")
close(z)

w <- file(f, "rb")
cat(readBin(w, "raw", 2), "\n")
close(w)
