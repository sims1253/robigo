# A parse error under keep.source = TRUE names the srcfile: parseError()'s
# message (getParseFilename) and the classed conditions' filename
# (getFilename): "<text>", the file's path; "<input>" without a srcfile
# environment or with an empty filename.
show <- function(e) { cat(class(e)[1], "|", conditionMessage(e), "|", e$filename, e$lineno, e$colno, "\n") }
for (ks in c(TRUE, FALSE)) {
  tryCatch(parse(text = "x <- 1\ny <- 1e", keep.source = ks), error = show)
  tryCatch(parse(text = "x <- '\\q'", keep.source = ks), error = show)
  f <- tempfile(fileext = ".R"); writeLines(c("a <- 1", "b <- '\\q'"), f)
  tryCatch(parse(f, keep.source = ks), error = function(e) cat(class(e)[1], "|", sub(f, "FILE", conditionMessage(e), fixed = TRUE), "|", identical(e$filename, f), e$lineno, e$colno, "\n"))
  tryCatch(parse(text = "a <- 1\n b <- 0x", srcfile = srcfilecopy("", "a"), keep.source = ks), error = show)
  tryCatch(parse(text = "a <- '\\q'", srcfile = srcfilecopy("", "a"), keep.source = ks), error = show)
  tryCatch(parse(text = 'r"abc"', keep.source = ks), error = show)
}
# a directive inside a string names no file
tryCatch(parse(text = "x <- '#line 9 \"s.R\"\n'\ny <- '\\q'", keep.source = TRUE), error = show)
