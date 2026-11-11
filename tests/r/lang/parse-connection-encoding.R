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
unlink(tf)
