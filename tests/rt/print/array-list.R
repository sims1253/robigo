# array-like lists: each element's summary as a character matrix or array
matrix(list(1, "a", TRUE, NULL), 2)
matrix(list(1:3, letters, c(TRUE, FALSE), list(1, 2)), 2)
matrix(list(1L, 2.5, 1i, NA), 2)
matrix(list(quote(f(x)), as.raw(1), complex(real = NA), NA_character_), 2)
# an object's element: its format() if one short string, else class,length
# (factor's format needs .Internal(format()), not ported yet: methods here)
format.foo <- function(x, ...) "FOO"
format.bar <- function(x, ...) strrep("b", 120)
format.baz <- function(x, ...) c("a", "b")
o <- function(cl, n) structure(seq_len(n), class = cl)
matrix(list(o("foo", 1), o("bar", 3), o("baz", 2), 1), 2)
l <- matrix(list(1, 2, 3, 4), 2, dimnames = list(c("a", "b"), c("x", "y"))); l
dimnames(l) <- list(R = c("a", "b"), C = c("x", "y")); l
print(l, right = TRUE)
matrix(list(strrep("x", 150), "short"), 1)
matrix(list(paste(rep("é", 120), collapse = ""), 1), 1)
array(list(1, "a", TRUE, NULL, 1:2, "b", 3, 4), c(2, 2, 2))
structure(list(1, 2), dim = c(1L, 2L), extra = "attr")
p <- pairlist(1, "a", TRUE, NULL)
dim(p) <- c(2L, 2L)
p
dimnames(p) <- list(c("r1", "r2"), c("c1", "c2"))
p
