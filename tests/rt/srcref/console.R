# options(keep.source = TRUE) in a script: the console (R_Parse1Buffer)
# keeps source references from the next statement on, its srcfile a
# "srcfilecopy" of the statement (filename "", lines the console buffer).
# Invalid values are options.c's errors; sys.source keeps them too.
options(keep.source = TRUE)
f <- function(x) {
  x + 1  # one
}
print(f)
f
sf <- attr(attr(f, "srcref"), "srcfile")
print(class(sf)); print(sort(ls(sf))); print(sf$filename); print(sf$lines)
print(unclass(attr(f, "srcref")))
print(lapply(attr(body(f), "srcref"), unclass))
g <- function() 1; h <- function(y)   y
print(unclass(attr(h, "srcref"))); h
x <- 1
print(attributes(quote(x)))
{ a <- 1
  b <- 2 }
k <- function(s = "ä\tz") s
k
print(unclass(attr(k, "srcref")))
print(deparse(f)); print(body(f))
options(keep.source = FALSE)
m <- function(x) x  # gone
m
tryCatch(options(keep.source = NA), error = function(e) print(conditionMessage(e)))
tryCatch(options(keep.source = 1), error = function(e) print(conditionMessage(e)))
options(keep.source = TRUE)
r <- tryCatch(parse(text = "'\\q'"), error = function(e) e$filename); print(r)
tf <- tempfile(fileext = ".R"); writeLines(c("s1 <- function(z) {", "  z  # from sys.source", "}"), tf)
en <- new.env(); sys.source(tf, envir = en, keep.source = TRUE)
cat(deparse(en$s1, control = "useSource"), sep = "\n"); print(identical(attr(attr(en$s1, "srcref"), "srcfile")$filename, tf))
en0 <- new.env(); sys.source(tf, envir = en0, keep.source = FALSE); print(attributes(en0$s1))
