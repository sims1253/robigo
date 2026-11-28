# A carriage return read from a file connection ends the parser's line:
# con_getc reads through Rconn_fgetc, which gives CR and CR LF as a newline
# (and a CR after a CR as one too), so "\<CR>" is an escaped newline, a
# \u{...} cut by a CR ends at the next line's column 0, and what parse(con,
# n = 1) leaves unread is counted in the connection's own bytes (#1017).
options(keep.source = FALSE)
bs <- "\\"
probe <- function(prefix, esc, end, modes) {
  txt <- paste0(prefix, '"', bs, esc, end, '";TAIL')
  for (mode in modes) {
    if (mode == "con") {
      f <- tempfile(); writeLines(txt, f, useBytes = TRUE); con <- file(f, "r")
      x <- tryCatch(parse(con, n = 1), error = identity)
      rest <- deparse(readLines(con, warn = FALSE))
      close(con)
    } else {
      t <- txt
      if (mode == "native") Encoding(t) <- "unknown"
      x <- tryCatch(parse(text = t), error = identity)
      rest <- ""
    }
    shown <- paste(as.character(charToRaw(txt)), collapse = " ")
    if (inherits(x, "error")) {
      msg <- conditionMessage(x)
      if (class(x)[1] == "invalidUnicode" && grepl("^u", esc)) msg <- "(GNU R's uninitialized line number: message omitted)"
      cat(sprintf("%-6s", mode), shown, "|", class(x)[1], x$lineno, x$colno, "|", paste(as.character(charToRaw(msg)), collapse = " "), "|", rest, "\n")
    } else cat(sprintf("%-6s", mode), shown, "| OK |", rest, "\n")
  }
}
# B: a carriage return read from a file connection ends the line for the
# parser (the escaped "\<CR>" is an escaped newline; a \u{...} cut by it is
# at the next line's column 0)
for (prefix in c("", "y ", "\t", "\n"))
  for (esc in c("", "u{1", "U{1", "u{1234", "U{12345678"))
    probe(prefix, esc, "\r", "con")
# CRLF and CR line ends in a file: parse() with source references (which
# reads the file's lines), and parse(con, n = 1) leaving the rest unread
hex <- function(s) paste(as.character(charToRaw(s)), collapse = " ")
for (txt in c("x <- 1\r\ny <- 'a\r\nb'\r\nz", "x <- 1\ry <- 2\r\r\nz <- 3", "f <- function(a)\r\n{\r\n  a\r\n}\r\n")) {
  f <- tempfile(); writeLines(txt, f, sep = "", useBytes = TRUE)
  e <- parse(f, keep.source = TRUE)
  cat(length(e), "|", vapply(e, function(x) paste(deparse(x), collapse = " "), ""), "\n")
  cat("  srcref:", vapply(attr(e, "srcref"), function(s) paste(as.integer(s), collapse = ","), ""), "\n")
  cat("  text:", vapply(attr(e, "srcref"), function(s) hex(paste(as.character(s), collapse = "|")), ""), "\n")
  con <- file(f, "r"); r <- parse(con, n = 1); cat("  n=1 rest:", deparse(readLines(con, warn = FALSE)), "\n"); close(con)
}
# what stays unread after a statement or an error at each kind of line end
rest <- function(txt) {
  f <- tempfile(); writeLines(txt, f, sep = "", useBytes = TRUE); con <- file(f, "r")
  r <- tryCatch(deparse(parse(con, n = 1)[[1]]), error = function(e) "error")
  l <- readLines(con, warn = FALSE)
  cat(deparse(txt), r, "|", length(l), hex(paste(l, collapse = "|")), "\n"); close(con)
}
for (txt in c("1\rx", "1\r\nx", "1\r\rx", "1\r\r\nx", "1\r", "1\r\r", "x <- )\r\nNEXT", "x <- )\rNEXT", "y z\r\nNEXT",
              "'a\rb'\rNEXT", "1 +\r2\rNEXT", "f(\r\n1)\r\nNEXT", "# c\r\n1\r\nNEXT"))
  rest(txt)
