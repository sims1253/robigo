# `#line N "file"` (gram.y's processLineDirective): the line numbers go on
# from N, the parsed lines do not; the file is a "srcfilealias" whose
# `original` is the parse's srcfile. CRLF line ends, read from a file.
show <- function(x) { a <- attributes(x); for (n in names(a)) if (n != "srcfile") { cat(n, ":\n"); if (is.list(a[[n]])) for (s in a[[n]]) print(as.integer(unclass(s))) else print(as.integer(unclass(a[[n]]))) } }
tf <- tempfile(fileext = ".R"); writeLines(c("a <- 1", "b <- 2"), tf, sep = "\r\n")
cr <- parse(tf, keep.source = TRUE); show(cr)
print(class(attr(cr, "srcfile"))); print(identical(attr(cr, "srcfile")$filename, tf))
ln <- parse(text = "a <- 1\n#line 40 \"other.R\"\nb <- 2\nc <- 3", keep.source = TRUE); show(ln)
print(sapply(attr(ln, "srcref"), function(s) attr(s, "srcfile")$filename))
al <- attr(attr(ln, "srcref")[[2]], "srcfile")
print(class(al)); print(identical(al$original, attr(ln, "srcfile")))
# the same name again makes no new alias; a directive inside a string is none
ln2 <- parse(text = "#line 5 \"<text>\"\na <- 1\nb <- '\n#line 9 \"s.R\"\n'", keep.source = TRUE)
print(sapply(attr(ln2, "srcref"), function(s) class(attr(s, "srcfile"))[1])); show(ln2)
