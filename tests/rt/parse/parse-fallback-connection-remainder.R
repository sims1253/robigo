# parse(con, n = 1) on an open file connection after a grammar (yyerror)
# error: what stays unread is what GNU R's lexer had not read yet, through
# the error token and what it looked at past it (#1010): `<<` (its failed
# nextchar), `%abc` and a newline (SpecialValue reads the newline), a
# rejected multibyte token (read whole), `_` (nothing past it).
rem <- function(txt) {
  f <- tempfile(); writeLines(c(txt, "NEXT"), f, useBytes = TRUE)
  con <- file(f, "r")
  r <- tryCatch(parse(con, n = 1), error = function(e) class(e)[1])
  cat(sprintf("%-22s", deparse(txt)), deparse(r), "|", deparse(readLines(con, warn = FALSE)), "\n")
  close(con)
}
emoji <- intToUtf8(0x1F642)
conn <- c("y ", "1 + ", "f(", "function(", "", "x[")
act <- c("1 |> f(_()) ", "f(\"\"= ", "_ + 1 ", "_ ", "f(_) ")
for (p in c(conn, act)) {
  rem(paste0(p, "<<;TAIL"))
  rem(paste0(p, "%abc\n;TAIL"))
  rem(paste0(p, emoji, ";TAIL"))
}
for (p in c("y ", "function(")) rem(paste0(p, "_;TAIL"))
