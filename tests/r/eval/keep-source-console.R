# Rscript reads the file through the console REPL. Once
# options(keep.source = TRUE) is set, each statement read afterwards keeps
# its source references (R: gram.y:R_Parse1Buffer).
f <- function(x) {
  x + 1 # dropped: keep.source is FALSE in Rscript
}
print(f)
print(is.null(attr(f, "srcref")))
options(keep.source = TRUE)
h <- function(y) {
  y - 1 # kept
}
print(h)
r <- attr(h, "srcref")
print(as.integer(r))
sf <- attr(r, "srcfile")
print(class(sf))
print(sf$filename)
print(sf$lines)
print(getSrcLines(sf, 1, 3))
print(as.character(r))
# Braces outside a function keep their references too.
b <- quote({
  1
  2
})
print(length(attr(b, "srcref")))
# Two statements on one line: each is its own buffer.
g1 <- function() 1; g2 <- function()   2
print(g1)
print(g2)
print(attr(g2, "srcref")[c(1, 2, 4)])
print(attr(attr(g2, "srcref"), "srcfile")$lines)
# A statement without functions or braces makes no srcfile.
v <- 1 + 2
print(v)
options(keep.source = FALSE)
k <- function(z) {
  z # gone again
}
print(k)
