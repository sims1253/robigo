# S3 dispatch in a loop: UseMethod on a class attribute, on implicit
# classes (a matrix, a double) and to the default method; NextMethod;
# internal generics ([, length, $) and the Ops group on classed objects.
n <- 20000
area <- function(s, ...) UseMethod("area")
area.sq <- function(s, ...) s$side^2
area.rect <- function(s, ...) NextMethod() * 2
area.matrix <- function(s, ...) 1
area.numeric <- function(s, ...) 2
area.default <- function(s, ...) 0
sq <- structure(list(side = 2), class = "sq")
rc <- structure(list(side = 3), class = c("rect", "sq"))
m <- matrix(1:4, 2)
`[.wrap` <- function(x, i) unclass(x)[i] + 1
length.wrap <- function(x) 99L
`$.rec` <- function(x, name) 7
Ops.money <- function(e1, e2) {
  v <- get(.Generic)(unclass(e1), unclass(e2))
  if (.Generic %in% c("+", "-", "*", "/")) structure(v, class = "money") else v
}
w <- structure(c(1, 2, 3), class = "wrap")
r <- structure(list(), class = "rec")
mo <- structure(5, class = "money")
s <- 0
for (i in 1:n) {
  s <- s + area(sq) + area(rc) + area(m) + area(1.5) + area("a")
  s <- s + w[2] + length(w) + r$foo
  mo2 <- mo + i
  s <- s + unclass(mo2) + (mo2 > 3)
}
cat(s, "\n")
