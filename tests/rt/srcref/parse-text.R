# parse(text =, keep.source = TRUE): the expression vector's srcref,
# srcfile and wholeSrcref; a function's srcref (its 4th element); the
# srcref lists of `{` (gram.y's makeSrcref, xxdefun, attachSrcrefs).
show <- function(x) { a <- attributes(x); for (n in names(a)) if (n != "srcfile") { cat(n, ":\n"); if (is.list(a[[n]])) for (s in a[[n]]) print(as.integer(unclass(s))) else print(as.integer(unclass(a[[n]]))) } }
e <- parse(text = "f <- function(x) {\n  y <- x + 1\n  y * 2\n}\ng <- 3", keep.source = TRUE)
show(e)
show(e[[1]][[3]][[3]])
print(as.integer(unclass(e[[1]][[3]][[4]])))
print(class(attr(e, "srcfile"))); print(attr(e, "srcfile")$filename)
print(identical(attr(e[[1]][[3]][[4]], "srcfile"), attr(e, "srcfile")))
# a tab: columns to the next multiple of 8
tb <- parse(text = "if (TRUE) {\n\tx <- 1\n}", keep.source = TRUE); show(tb); show(tb[[1]][[3]])
# multibyte characters: a column each, their bytes counted twice inside
# strings (StringValue's re-read), raw strings too
p <- parse(text = 'x <- "ä"; y <- r"(ä)"; z <- 1', keep.source = TRUE)
for (s in attr(p, "srcref")) print(as.integer(unclass(s)))
p <- parse(text = 'f <- function() r"[a\tä]"\ng <- function() 1', keep.source = TRUE)
for (s in attr(p, "srcref")) print(as.integer(unclass(s)))
print(as.integer(unclass(attr(eval(p[[2]][[3]]), "srcref"))))
# `ä` in a name and in a comment
p <- parse(text = "ä <- function(b) b # ü\nä(1)", keep.source = TRUE)
for (s in attr(p, "srcref")) print(as.integer(unclass(s)))
# keep.source = FALSE: none of it
p0 <- parse(text = "f <- function(x) { x }", keep.source = FALSE)
print(attributes(p0)); print(p0[[1]][[3]][[4]]); print(attributes(p0[[1]][[3]][[3]]))
