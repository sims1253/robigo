# A connection opened at creation with an encoding iconv cannot convert:
# set_iconv_error destroys it, then checked_open's cleanup destroys it
# again and getConnection's "invalid connection" replaces the message
# (R: connections.c:checked_open, cend_con_destroy, set_iconv_error).
nopen <- nrow(showConnections())
tmp <- tempfile()
cat("1\n2\n", file = tmp)
r <- try(file(tmp, open = "r", encoding = "unknown"))
cat(r)
r <- try(file(tmp, open = "rt", encoding = "no-such-enc"))
w <- tempfile()
r <- try(file(w, open = "w", encoding = "no-such-enc"))
nrow(showConnections()) == nopen
# a binary open never converts
con <- file(tmp, open = "rb", encoding = "no-such-enc")
readBin(con, "raw", 2L)
close(con)
# open() of an existing connection reports the conversion itself
con <- file(tmp, encoding = "no-such-enc")
r <- try(open(con))
nrow(showConnections()) == nopen
# a valid encoding still opens
con <- file(tmp, open = "r", encoding = "latin1")
readLines(con)
close(con)
# compressed-file connections check the conversion too
r <- try(gzfile(w, open = "wt", encoding = "no-such-enc"))
r <- try(bzfile(w, open = "wt", encoding = "no-such-enc"))
r <- try(xzfile(w, open = "wt", encoding = "no-such-enc"))
# a mode without "b" is text
r <- try(gzfile(w, open = "w", encoding = "no-such-enc"))
r <- try(bzfile(w, open = "a", encoding = "no-such-enc"))
r <- try(gzfile(tmp, open = "r", encoding = "no-such-enc"))
con <- gzfile(w, open = "w"); summary(con)$text; close(con)
con <- gzfile(w, open = "wb", encoding = "no-such-enc"); summary(con)$text; close(con)
nrow(showConnections()) == nopen
unlink(c(tmp, w))
