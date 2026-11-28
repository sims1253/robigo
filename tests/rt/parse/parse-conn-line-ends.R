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

# what Rconn_fgetc holds (con->save) across parses: the character after a
# CR is read at once, a second CR is held as a newline, the end of input as
# the end; pushed-back input is given as it is
wr <- function(txt) { f <- tempfile(); writeLines(txt, f, sep = "", useBytes = TRUE); f }
# S1: consecutive parses over a held newline (CR CR LF), and over each line end
for (txt in c("1\r\r\n2\n", "1\r2\r3\n", "1\r\r2\n", "1\r\n2\r\n3", "1\r\r\r2\n")) {
  con <- file(wr(txt), "r")
  got <- character()
  repeat { e <- tryCatch(parse(con, n = 1), error = function(e) "error"); if (!length(e) || identical(e, "error")) break; got <- c(got, deparse(e[[1]])) }
  cat(deparse(txt), "|", got, "| left", deparse(readLines(con, warn = FALSE)), "\n"); close(con)
}
# S2: the character after a CR is read; readChar bypasses the held one
for (txt in c("1\r2\n", "1\r\r2\n", "1\r")) {
  con <- file(wr(txt), "rb"); p <- parse(con, n = 1)
  cat(deparse(txt), "| readChar", deparse(readChar(con, 1L, useBytes = TRUE)), "| readLines", deparse(readLines(con, warn = FALSE)), "\n"); close(con)
}
# S3: pushed-back input keeps its CR (pushBack text is given as it is)
con <- file(wr("NEXT\n"), "r"); pushBack("'a\rb'", con); e <- parse(con); cat("pushBack file:", hex(e[[1]]), length(e), "\n"); close(con)
tc <- textConnection("NEXT"); pushBack("'a\rb'", tc); e <- parse(tc); cat("pushBack text:", hex(e[[1]]), length(e), "\n"); close(tc)
# S4: a held end of input, then more appended
f <- wr("1\r"); con <- file(f, "r"); p <- parse(con, n = 1)
a <- file(f, "a"); writeLines("2", a); close(a)
cat("S4:", deparse(readLines(con, warn = FALSE)), deparse(readLines(con, warn = FALSE)), "\n"); close(con)
# a parse that runs out of input reads a held end too: nothing is held after
held_end <- function(txt, n) {
  f <- wr(txt); con <- file(f, "r"); p <- if (is.na(n)) parse(con) else parse(con, n = n)
  a <- file(f, "a"); writeLines("2", a); close(a)
  cat(deparse(txt), n, length(p), "|", deparse(readLines(con, warn = FALSE)), deparse(readLines(con, warn = FALSE)), "\n"); close(con)
}
held_end("\r", 1); held_end("1\r", 1); held_end("1\r", 2); held_end("1\r", NA); held_end("\r\r", 1); held_end("1\r\r", 1)
