# A closure with a srcref prints its source text (print.function's
# useSource), comments kept; deparse() and body() do not use it.
e <- parse(text = "f <- function(x) {\n  # a comment\n  y <- x + 1\n  y * 2\n}", keep.source = TRUE)
f <- eval(e[[1]]); print(f); f
print(deparse(f)); print(f, useSource = FALSE)
print(body(f)); print(names(attributes(body(f))))
print(deparse(f, control = "useSource"))
cat(as.character(attr(e, "wholeSrcref")), sep = "\n")
e <- parse(text = "f <- function(x) {\n  # c\n  x\n}\ng <- 3 # tail", keep.source = TRUE)
print(e); print(e[[1]]); print(e[1]); print(deparse(e))
# the srcfile's lines changed: the print follows them
f <- eval(e[[1]])
sf <- attr(attr(f, "srcref"), "srcfile")
sf$lines <- sub("# c", "# changed", sf$lines)
f
# keep.source = FALSE afterwards: deparsed
f2 <- eval(parse(text = "function(x)   x  # gone", keep.source = FALSE)[[1]]); f2
