# R: connections.c:xzfile_read — a decoder error can come in the same
# lzma_code call that fills a read's request: that read warns and still
# gives all it asked for, and the next read meets the sticky error. Which
# reads those are follows liblzma's progress within each call, so robigo
# reads a failing file as R does: BUFSIZ input chunks, R's request sizes.
mkz <- function(bytes, at) {
  z <- memCompress(bytes, "xz")
  z[length(z) - at] <- as.raw(255)
  f <- tempfile(fileext = ".xz"); writeBin(z, f); f
}
f <- mkz(as.raw(rep(65L, 4096)), 20)
con <- xzfile(f, "rb")
a <- readBin(con, "raw", 4096); print(length(a))
b <- readBin(con, "raw", 1); print(length(b))
close(con)
con <- xzfile(f, "rb"); print(length(readBin(con, "raw", 5000))); close(con)
con <- xzfile(f, "rb"); print(length(readBin(con, "integer", 1024))); print(readBin(con, "integer", 1)); close(con)
con <- xzfile(f, "rb"); print(length(readBin(con, "integer", 3000, size = 2))); close(con)
con <- xzfile(f, "rb"); print(length(readBin(con, "double", 512))); print(length(readBin(con, "double", 1))); close(con)
con <- xzfile(f, "r"); l <- readLines(con, warn = FALSE); print(nchar(l)); close(con)
# text: a 4096-byte buffer fill that reaches the end
t <- mkz(charToRaw(paste0(strrep("x", 4095), "\n")), 20)
con <- xzfile(t, "r"); print(nchar(readLines(con))); print(readLines(con)); close(con)
print(nchar(readLines(xzfile(t, encoding = "latin1"))))
print(nchar(scan(xzfile(t), what = "", quiet = TRUE)))
# a CRLF split by the buffer, two fills, a last fill that is full
cr <- mkz(charToRaw(paste0(strrep("y", 4095), "\r\n", strrep("z", 4094), "\n")), 20)
print(nchar(readLines(xzfile(cr))))
# more input than one BUFSIZ: an incompressible file
x <- numeric(30000); s <- 12345
for (k in seq_along(x)) { s <- (69069 * s + 1) %% 4294967296; x[k] <- s %/% 16777216 }
say <- function(expr) withCallingHandlers(expr, warning = function(w) {
  cat("  warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning")
})
z0 <- memCompress(as.raw(x), "xz"); g <- tempfile(fileext = ".xz")
for (cut in c(0, 9000, 16384, 20000)) {
  z <- z0[seq_len(length(z0) - cut)]
  if (!cut) z[length(z) - 20] <- as.raw(255)
  writeBin(z, g)
  con <- xzfile(g, "rb"); n <- 0
  repeat { k <- length(say(readBin(con, "raw", 8096))); n <- n + k; if (k < 8096) break }
  close(con); cat("raw", cut, n, "\n")
  con <- xzfile(g, "rb"); cat("block", cut, length(say(readBin(con, "raw", 30000))), "\n"); close(con)
  con <- xzfile(g, "rb"); cat("int", cut, length(say(readBin(con, "integer", 9000))), "\n"); close(con)
}
w <- mkz(charToRaw(paste(sprintf("%s", vapply(1:500, function(k)
  rawToChar(as.raw(97 + x[(k * 50):(k * 50 + 39)] %% 26)), "")), collapse = "\n")), 20)
print(length(readLines(xzfile(w), warn = FALSE)))
unlink(c(f, t, cr, g, w))
