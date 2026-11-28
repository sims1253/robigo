# An unrecognized escape whose character is not ASCII: gram.y's StringValue
# reads one byte after the backslash, so the message and the condition's
# value carry that lone byte (native encoding), the column counts it, and a
# connection has read no further (#1016). The byte may lead a multibyte
# character or be invalid UTF-8 by itself.
options(keep.source = FALSE)
bs <- "\\"
hex <- function(s) paste(as.character(charToRaw(s)), collapse = " ")
probe <- function(prefix, tail) {
  txt <- paste0(prefix, '"', bs, tail, '";TAIL')
  for (mode in c("text", "native", "con")) {
    if (mode == "con") {
      f <- tempfile(); writeLines(txt, f, useBytes = TRUE); con <- file(f, "r")
      x <- tryCatch(parse(con, n = 1), error = identity)
      rest <- hex(paste(readLines(con, warn = FALSE), collapse = "|"))
      close(con)
    } else {
      t <- txt
      if (mode == "native") Encoding(t) <- "unknown"
      x <- tryCatch(parse(text = t), error = identity)
      rest <- ""
    }
    if (inherits(x, "error"))
      cat(sprintf("%-6s", mode), hex(txt), "|", class(x)[1], x$lineno, x$colno, "|", hex(conditionMessage(x)),
          "| value", hex(x$value), Encoding(x$value), "|", rest, "\n")
    else cat(sprintf("%-6s", mode), hex(txt), "| OK |", rest, "\n")
  }
}
for (prefix in c("", "y ", "\t", "\n"))
  for (ch in c(233L, 20013L, 128512L)) probe(prefix, intToUtf8(ch))
# an invalid UTF-8 byte after the backslash, alone and before ASCII
for (prefix in c("", "y ")) {
  probe(prefix, "\xff")
  probe(prefix, "\xc3x")
  probe(prefix, "\x80")
}
# str2lang's condition
e <- tryCatch(str2lang(paste0('"', bs, intToUtf8(233), '"')), error = identity)
cat(class(e)[1], hex(conditionMessage(e)), "\n")
