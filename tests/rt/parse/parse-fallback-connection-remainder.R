# Not fixed (issue #1010; recorded, not ratcheted): parse(con, n = 1) on an
# open file connection after a grammar (yyerror) error leaves a different
# remainder unread than GNU R for four token families: `<<`, `%abc` and a
# newline, a rejected multibyte token, `_` after `y ` or `function(`. The
# grammar-error fallback (source.rs parse_text) predates the parseError
# conditions; ratchet this when #1010 is fixed.
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
