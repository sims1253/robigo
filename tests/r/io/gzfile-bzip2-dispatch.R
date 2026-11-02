# R: connections.c:do_gzfile — gzfile() in read mode checks the file's
# magic and opens a bzfile connection for bzip2 data (comp_type_from_file).
p <- tempfile(fileext = ".bz2")
zz <- bzfile(p, "w")
writeLines(c("one", "two"), zz)
close(zz)
con <- gzfile(p, "rt")
print(summary(con)$class)
print(readLines(con))
close(con)
# a plain file still reads transparently through gzfile
p2 <- tempfile()
writeLines("plain", p2)
print(readLines(gzfile(p2)))
