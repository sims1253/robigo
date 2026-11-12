# A connection's `encoding` re-encodes its input before parse(),
# readLines() and scan() see it (R: connections.c set_iconv/dummy_fgetc).
tf <- tempfile()
hide <- function(m) sub(tf, "<tf>", m, fixed = TRUE)
writeBin(c(charToRaw('x <- "fa'), as.raw(0xE7), charToRaw('ile"\n')), tf)
source(tf, encoding = "latin1")
print(x); print(Encoding(x)); print(charToRaw(x))
x <- NULL
source(tf, encoding = c("UTF-8", "latin1"))
print(x)
r <- tryCatch(parse(tf), error = function(e) conditionMessage(e)); print(hide(r))
r <- withCallingHandlers(
    tryCatch(parse(tf, encoding = "latin1"), error = function(e) conditionMessage(e)),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(hide(r))
tf2 <- tempfile(); writeLines("z <- 2", tf2)
r <- withCallingHandlers(parse(tf2, encoding = "bogus"),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
print(r); unlink(tf2)
print(parse(file(tf, encoding = "latin1")))

# readLines() and scan()
writeBin(c(charToRaw("a"), as.raw(0xE7), charToRaw("b\nsecond line\n")), tf)
l <- readLines(file(tf, encoding = "latin1")); print(l); print(Encoding(l))
print(scan(file(tf, encoding = "latin1"), what = "", sep = "\n", quiet = TRUE))
r <- withCallingHandlers(readLines(file(tf, encoding = "UTF-8")),
    warning = function(w) { cat("warning:", hide(conditionMessage(w)), "\n"); invokeRestart("muffleWarning") })
print(r)
op <- options(warn = 2)
r <- tryCatch(readLines(file(tf, encoding = "UTF-8")), error = function(e) hide(conditionMessage(e)))
options(op)
print(r)

# a UTF-8 byte-order mark is dropped
writeBin(c(as.raw(c(0xEF, 0xBB, 0xBF)), charToRaw("y <- 1\n")), tf)
print(readLines(file(tf, encoding = "UTF-8-BOM")))
print(parse(file(tf, encoding = "UTF-8-BOM")))

# longer than the 25-byte conversion buffer, with characters across it
writeBin(rep(c(charToRaw("ab"), as.raw(0xE9)), 40), tf)
l <- readLines(file(tf, encoding = "latin1"), warn = FALSE)
print(nchar(l)); print(substr(l, 1, 12))

# parse(n =) converts and reads no further than its n-th statement
writeBin(c(charToRaw("1\n"), charToRaw(strrep(" ", 100)), as.raw(0xFF)), tf)
r <- withCallingHandlers(parse(file(tf, encoding = "UTF-8"), n = 1),
    warning = function(w) { cat("warning:", hide(conditionMessage(w)), "\n"); invokeRestart("muffleWarning") })
print(r)
op <- options(warn = 2)
print(parse(file(tf, encoding = "UTF-8"), n = 1))
options(op)
show <- function(con, n) { print(parse(con, n = n)); print(readLines(con)) }
writeLines(c("1; 2", "3", "4", "5"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
con <- file(tf, "r", encoding = "latin1"); show(con, 1); close(con)
con <- file(tf, "r"); print(parse(con, n = 2)); print(readLines(con, 1)); show(con, 1); close(con)
writeLines(c('x <- "a;b"; y <- 2', "z # c;d", "w"), tf)
for (n in 1:3) { con <- file(tf, "r"); show(con, n); close(con) }
writeLines(c("", "# only a comment", "", "a", "b"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
writeLines(c("1", "2"), tf)
con <- file(tf, "r"); show(con, 5); close(con)
writeLines(c("f <- function(x) {", "  x; x", "}", "g"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
writeLines(c("f(1,", "2)", "x"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
writeLines(c("1; '\\q'", "3"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
writeBin(charToRaw("1\r\n2\r\n3\r\n"), tf)
con <- file(tf, "r"); show(con, 1); close(con)
writeLines(c("q", "r"), tf)
con <- file(tf, "r"); pushBack(c("p1; p2", "p3"), con); show(con, 1); close(con)
con <- textConnection(c("1;2", "3")); show(con, 1); close(con)

# a compressed file's encoding (R: connections.c:gzfile_open calls set_iconv)
tf <- tempfile(fileext = ".gz")
con <- gzfile(tf, "wb"); writeBin(c(charToRaw('x <- "fa'), as.raw(0xE7), charToRaw('ile"\n')), con); close(con)
x <- NULL
source(tf, encoding = "latin1"); print(x); print(charToRaw(x))
print(readLines(gzfile(tf, encoding = "latin1")))
# gzfile() opens in "rb" by default, so parse() reads the bytes unconverted
print(tryCatch(parse(gzfile(tf, encoding = "latin1")), error = function(e) conditionMessage(e)))
con <- gzfile(tf, "r"); print(summary(con)$text); close(con)
con <- gzfile(tf, "rb"); print(summary(con)$text); close(con)
con <- gzfile(tf, "r"); print(tryCatch(readBin(con, "raw", 3), error = function(e) conditionMessage(e))); close(con)
unlink(tf)
# dummy_fgetc reads an unbuffered connection (a fifo) only up to a line
# end, a buffered one (a regular file) 25 bytes at a time
# (R: connections.c:593, `!con->buff`)
two_lines <- function(con) for (k in 1:2) print(withCallingHandlers(readLines(con, 1),
    warning = function(w) { cat("warning at line", k, "\n"); invokeRestart("muffleWarning") }))
bytes <- c(charToRaw("ab\ncd"), as.raw(0xE7), charToRaw("\n"))
tf <- tempfile()
writeBin(bytes, tf)
con <- file(tf, encoding = "UTF-8"); open(con); two_lines(con); close(con)
unlink(tf)
w <- fifo(tf, "w+b"); writeBin(bytes, w)
con <- fifo(tf, "r", encoding = "UTF-8"); two_lines(con); close(con); close(w)
unlink(tf)
