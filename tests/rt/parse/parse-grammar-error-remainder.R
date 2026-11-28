# parse(con, n = 1) on an open file connection after a grammar (yyerror)
# error, for each kind of error token in two contexts and three followers:
# the connection keeps what GNU R's lexer had not read (the token, and the
# byte or character it looked at past it: a symbol's or number's end, an
# operator that could have been longer).
rem <- function(txt) {
  f <- tempfile(); writeLines(c(txt, "NEXT"), f, useBytes = TRUE)
  con <- file(f, "r")
  r <- tryCatch({parse(con, n = 1); "ok"}, error = function(e) class(e)[1])
  cat(sprintf("%-26s", deparse(txt)), r, "|", deparse(readLines(con, warn = FALSE)), "\n")
  close(con)
}
toks <- c("1", "12", "1L", "1i", "1.5", ".5", "0x1F", "0x1FL", "1e5", "1e", "abc", "a.b", "a_b", ".x", "TRUE", "NULL",
  "Inf", "NA_integer_", "'s'", '"s"', "`b`", 'r"(raw)"', "%in%", "<-", "<<-", "->", "->>", "<=", "<", ">=", ">",
  "==", "=", "!=", "!", "&&", "&", "||", "|", "|>", "::", ":::", ":", "+", "-", "*", "**", "/", "^", "~", "?", "@", "$",
  "if", "else", "for", "while", "repeat", "function", "\\(x)", "break", "next", "{", "}", "(", ")", "[", "[[", "]", "]]", ",", "...", "..1", "=>")
for (p in c("y ", "f(1) "))
  for (t in toks)
    for (after in c(";TAIL", "zTAIL", " TAIL"))
      rem(paste0(p, t, after))
# install("") in the lexer: an error without a position, after both backticks
# were read; and a grammar error before it, with n = 1 and after a first
# statement with n = 2
rem2 <- function(txt, n) {
  f <- tempfile(); writeLines(c(txt, "NEXT"), f, useBytes = TRUE)
  con <- file(f, "r")
  r <- tryCatch({parse(con, n = n); "ok"}, error = function(e) conditionMessage(e))
  cat(deparse(txt), n, deparse(r), "|", deparse(readLines(con, warn = FALSE)), "\n")
  close(con)
}
for (t in c("y z ``TAIL", "``TAIL", "y ``TAIL", "f(``)TAIL", "1e", "y 1e", "0x", "1.5e+")) rem2(t, 1)
rem2("a\ny z ``TAIL", 2)
rem2("a\n``TAIL", 2)
