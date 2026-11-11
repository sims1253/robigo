# parse(text=), str2lang() and str2expression() on strings holding bytes
# that are not valid UTF-8 (gram.y:mbcs_get_next): an invalid character
# raises "invalid multibyte character in parser", or ends the input when
# its bytes run past the text; comments keep any bytes.
show <- function(r) {
  if (inherits(r, "error")) {
    cat(paste(class(r), collapse = " "), "\n")
    writeLines(conditionMessage(r))
    if (inherits(r, "parseError")) print(unlist(r[c("lineno", "colno")]))
  } else print(r)
}
f <- function(s) show(tryCatch(parse(text = s, keep.source = FALSE), error = function(e) e))
f("zz \xff yy")
f("zz \xff yy + 1 + 2")
f("x <- 1\n\x80 + 2")
f("ab\xffcd")
f("12\xffcd")
f("f(\xff)")
f("x <- 1; zz \xff yy")
f("x\n\xff")
f("1 # \xff\n2")
f("\"a\xffb\"")
f("`a\xff`")
f("r\"(a\xffb)\"")
f("x <- 1 +\n  \xc3")
f("x + \xf0\x9f")
f(c("x <- 1", "zz \xff yy", "3"))
f("\x80")
f("a\x80\x80\x80\x80\x80\x80\x80")
f("x <- \"\xed\xa0\x80\"")
f("if (TRUE) \xff else 2")
# A character split by the error keeps its remaining bytes in the context.
f("x + \xc3\xb7 + 1")
f("1e\xc3\xa9 + 1")
g <- function(expr) show(tryCatch(expr, error = function(e) e))
g(str2lang("zz \xff yy + 1 + 2"))
g(str2lang("ab\xffcd"))
g(str2expression("1 # \xff"))
tf <- tempfile()
writeBin(charToRaw("x <- 1\nzz \xff yy + 1 + 2\n"), tf)
g(parse(tf, keep.source = FALSE))
writeBin(charToRaw("x <- 1 # \xff\n"), tf)
g(parse(tf, keep.source = FALSE))
