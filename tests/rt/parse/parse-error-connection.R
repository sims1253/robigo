# parse(con, n = 1) on an open file connection after a classed parse error:
# what stays unread is what GNU R's lexer had not read. A raiseLexError
# stops reading where it was raised; an escape's digit loop has read the
# character it rejected (xxgetc, then xxungetc into the lexer's buffer: the
# connection consumed it); a lexer error in the lookahead of a failing
# action comes first, with the same read position.
rem <- function(txt) {
  f <- tempfile(); writeLines(txt, f, useBytes = TRUE)
  con <- file(f, "r")
  r <- tryCatch(parse(con, n = 1), error = function(e) class(e)[1])
  cat(sprintf("%-28s", txt), deparse(r), "|", deparse(readLines(con)), "\n")
  close(con)
}
rem('y "\\q";TAIL')
rem('1 |> f(_) "\\q";TAIL')
rem('function(a, a "\\q";TAIL')
rem('f("" = 1 "\\q";TAIL')
rem('y "\\x";TAIL')
rem('1 |> f(_) "\\x";TAIL')
rem('function(a, a "\\x";TAIL')
rem('f("" = 1 "\\x";TAIL')
rem('y "\\xg";TAIL')
rem('1 |> f(_) "\\xg";TAIL')
rem('function(a, a "\\xg";TAIL')
rem('f("" = 1 "\\xg";TAIL')
rem('y "\\0";TAIL')
rem('1 |> f(_) "\\0";TAIL')
rem('function(a, a "\\0";TAIL')
rem('f("" = 1 "\\0";TAIL')
rem('y "\\400";TAIL')
rem('1 |> f(_) "\\400";TAIL')
rem('function(a, a "\\400";TAIL')
rem('f("" = 1 "\\400";TAIL')
rem('y "\\u{zz}";TAIL')
rem('1 |> f(_) "\\u{zz}";TAIL')
rem('function(a, a "\\u{zz}";TAIL')
rem('f("" = 1 "\\u{zz}";TAIL')
rem('y "\\U{110000}";TAIL')
rem('1 |> f(_) "\\U{110000}";TAIL')
rem('function(a, a "\\U{110000}";TAIL')
rem('f("" = 1 "\\U{110000}";TAIL')
rem('y `\\u1234`;TAIL')
rem('1 |> f(_) `\\u1234`;TAIL')
