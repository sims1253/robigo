# Gap: bzip2 file connections. cli's sysdata.rda (built during
# R CMD INSTALL via tools) is bzip2-compressed, so its install fails
# with "robigo: bzip2 compressed file connections are not implemented
# yet" without these.
p <- tempfile(fileext = ".bz2")
zz <- bzfile(p, "w")
writeLines(c("hello", "world"), zz)
close(zz)
print(file.exists(p))
zz <- bzfile(p, "r")
print(readLines(zz))
close(zz)
