# R: connections.c:xzfile_read warns at each read that asks for more than a
# failing decoder gives. Which reads those are depends on the reader: a
# text connection fills a 4096-byte buffer (buff_fill), an input
# conversion fills once more after the end (dummy_fgetc), readBin stops at
# a short block, and unserialize reads only what it needs.
mk <- function(lines, cut, eol = "\n") {
  f <- tempfile(); con <- xzfile(f, "wb")
  writeChar(paste0(paste(lines, collapse = eol), eol), con, eos = NULL); close(con)
  z <- readBin(f, "raw", file.size(f)); unlink(f)
  h <- tempfile(); writeBin(z[seq_len(length(z) - cut)], h); h
}
big <- mk(sprintf("line %05d", 1:3000), 30)
x <- readLines(xzfile(big)); print(length(x)); print(x[length(x)])
con <- xzfile(big, "r"); for (k in 1:5) { y <- readLines(con, n = 1000); cat(k, length(y), "\n") }; close(con)
s <- scan(xzfile(big), what = "", sep = "\n", quiet = TRUE); print(length(s))
h <- mk(c("a", "b", "c"), 20)
con <- xzfile(h, "rb"); print(readChar(con, 100)); print(readChar(con, 100)); close(con)
con <- xzfile(h, "rb"); print(readBin(con, "integer", 10)); close(con)
con <- xzfile(h, "rb"); print(readBin(con, "raw", 3)); print(readBin(con, "raw", 3)); print(readBin(con, "raw", 3)); close(con)
con <- xzfile(h, "r"); print(readLines(con, 1)); print(readLines(con)); print(readLines(con)); close(con)
print(readLines(xzfile(mk(c("one", "two", "three"), 20, eol = "\r\n"))))
print(readLines(xzfile(h, encoding = "latin1")))
print(length(readLines(xzfile(mk(sprintf("%04d", 1:2000), 25), encoding = "latin1"))))
r <- tempfile(); saveRDS(1:1000, r, compress = "xz"); z <- readBin(r, "raw", 1e5)
writeBin(z[seq_len(length(z) - 10)], r); print(sum(readRDS(r)))
print(utils::read.table(xzfile(mk(c("x y", "1 2", "3 4"), 20)), header = TRUE))
p <- mk(c("x <- 1", "y <- 2", "print(x + y)"), 20)
print(parse(xzfile(p)))
source(xzfile(p))
