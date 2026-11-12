# Parse error columns count bytes unless the input is known to be UTF-8
# (R: gram.y xxgetc, source.c do_parse).
msg <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
cat(msg(str2lang("é )")), "\n")
x <- "\xc3\xa9 )"
cat(msg(str2lang(x)), "\n")
cat(msg(parse(text = x)), "\n")
cat(msg(parse(text = x, encoding = "UTF-8")), "\n")
tf <- tempfile()
writeBin(charToRaw("\xc3\xa9\xc3\xa9 )\n"), tf)
cat(sub(tf, "<tf>", msg(parse(tf)), fixed = TRUE), "\n")
cat(sub(tf, "<tf>", msg(parse(tf, encoding = "UTF-8")), fixed = TRUE), "\n")
# a lexer condition's position too
x <- "\xc3\xa9 + '\\q'"
r <- tryCatch(str2lang(x), error = function(e) e)
cat(conditionMessage(r), r$lineno, r$colno, "\n")
r <- tryCatch(str2lang(enc2utf8(x)), error = function(e) e)
cat(conditionMessage(r), r$lineno, r$colno, "\n")
# a placeholder error that ends at `;`
x <- "\xc3\xa9 + _;"
r <- tryCatch(str2lang(x), error = function(e) e)
cat(conditionMessage(r), r$lineno, r$colno, "\n")
r <- tryCatch(str2lang(enc2utf8(x)), error = function(e) e)
cat(conditionMessage(r), r$lineno, r$colno, "\n")
r <- tryCatch(parse(text = x, encoding = "UTF-8"), error = function(e) e)
cat(conditionMessage(r), r$lineno, r$colno, "\n")
