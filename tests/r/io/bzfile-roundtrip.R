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
# a valid empty stream opens and reads nothing
e <- tempfile(fileext = ".bz2")
z <- bzfile(e, "wb")
close(z)
print(readLines(bzfile(e, "rb")))
# compress = 0 passes validation but BZ2_bzWriteOpen rejects it
p0 <- tempfile(fileext = ".bz2")
z0 <- tryCatch({ c <- bzfile(p0, "wb", compression = 0); close(c); "opened" },
  error = function(e) "E", warning = function(w) "W")
print(z0)
# trailing non-bzip2 bytes after a valid stream keep the decoded data
# and warn (R: connections.c:bzfile_read)
t <- tempfile(fileext = ".bz2")
zt <- bzfile(t, "wb")
writeLines("hello", zt)
close(zt)
con <- file(t, "ab")
writeBin(as.raw(c(0x67, 0x61, 0x72, 0x62, 0x61, 0x67, 0x65)), con)
close(con)
tw <- 0
cat(withCallingHandlers(
  paste(readLines(bzfile(t, "rb")), collapse = "|"),
  warning = function(x) { tw <<- 1; invokeRestart("muffleWarning") }), "\n")
print(tw)
# files truncated mid-header or mid-stream are silent and read empty
# (R: bzfile_read warns only for BZ_DATA_ERROR_MAGIC)
tr <- tempfile(fileext = ".bz2")
writeBin(as.raw(c(0x42, 0x5a, 0x68, 0x39)), tr)  # "BZh9" and nothing more
tw2 <- 0
cat(withCallingHandlers(
  paste(readLines(bzfile(tr, "rb")), collapse = "|"),
  warning = function(x) { tw2 <<- 1; invokeRestart("muffleWarning") }), "\n")
print(tw2)
tr2 <- tempfile(fileext = ".bz2")
writeBin(as.raw(c(0x42, 0x5a, 0x68, 0x39, 0x31, 0x41, 0x59, 0x26)), tr2)
cat(paste(readLines(bzfile(tr2, "rb")), collapse = "|"), "\n")
# a complete header with a wrong stream magic warns (PR#18768)
tr3 <- tempfile(fileext = ".bz2")
writeBin(as.raw(c(0x42, 0x5a, 0x68, 0x39, 1, 2, 3, 4, 5, 6, 7, 8)), tr3)
tw3 <- 0
lines3 <- withCallingHandlers(
  readLines(bzfile(tr3, "rb")),
  warning = function(x) { tw3 <<- 1; invokeRestart("muffleWarning") })
print(c(length(lines3), tw3))
