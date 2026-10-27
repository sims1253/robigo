"+.money" <- function(e1, e2) {
  structure(unclass(e1) + unclass(e2), class = "money")
}
print.money <- function(x, ...) cat("money:", unclass(x), "\n")
a <- structure(5, class = "money")
b <- structure(7, class = "money")
a + b
Ops.temp <- function(e1, e2) {
  v <- get(.Generic)(unclass(e1), unclass(e2))
  if (.Generic %in% c("+", "-", "*", "/")) structure(v, class = "temp") else v
}
t1 <- structure(10, class = "temp")
t2 <- structure(20, class = "temp")
unclass(t1 + t2)
t1 < t2
t1 == t2
length.myvec <- function(x) 99L
v <- structure(list(), class = "myvec")
length(v)
"[.myvec" <- function(x, i) paste("index", i)
v[3]
"$.rec" <- function(x, name) paste("field", name)
r <- structure(list(), class = "rec")
r$anything
as.character.dog <- function(x, ...) "DOG"
as.character(structure(1, class = "dog"))
