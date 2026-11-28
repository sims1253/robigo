# parse(con, n = 1) on an open file connection reads through con_getc, so a
# calling handler for what the parse signals finds the connection where the
# lexer stopped: the rest can be read from inside the handler, and is then
# gone afterwards (#1053).
hex <- function(s) paste(as.character(charToRaw(s)), collapse = " ")
rest <- function(con) deparse(hex(paste(readLines(con, warn = FALSE), collapse = "|")))
probe <- function(label, lines, class) {
  f <- tempfile(); writeLines(lines, f, useBytes = TRUE); con <- file(f, "r")
  h <- function(c) cat(label, class(c)[1], "in handler:", rest(con), "\n")
  hs <- list(h); names(hs) <- class
  r <- tryCatch(do.call(withCallingHandlers, c(list(quote(parse(con, n = 1))), hs)),
                error = function(e) "error", warning = function(w) "warning")
  cat(label, "result:", if (is.character(r)) r else deparse(r[[1]]), "| after:", rest(con), "\n")
  close(con)
}
# the lexer's classed conditions
probe("q", c('"\\q";TAIL', "NEXT"), "parseError")
probe("e", c(paste0('"\\', intToUtf8(233), '";TAIL'), "NEXT"), "parseError")
probe("u", c('"\\u{110000}";TAIL', "NEXT"), "parseError")
# a grammar error: parse()'s simpleError
probe("gram", c("x <- ) ;TAIL", "NEXT"), "error")
# no handler: the remainder is there after the error
f <- tempfile(); writeLines(c('"\\q";TAIL', "NEXT"), f); con <- file(f, "r")
try(parse(con, n = 1), silent = TRUE)
cat("plain after:", rest(con), "\n"); close(con)
