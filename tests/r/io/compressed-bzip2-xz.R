# R: connections.c:do_file — file() detects bzip2 and xz magic numbers and
# opens them through bzfile()/xzfile().
f <- tempfile()
con <- bzfile(f, "w")
writeLines(as.character(1:3), con)
close(con)
cat(readLines(f), sep = "|")
cat("\n")

g <- tempfile()
con <- xzfile(g, "w")
writeLines(as.character(1:3), con)
close(con)
cat(readLines(g), sep = "|")
cat("\n")
