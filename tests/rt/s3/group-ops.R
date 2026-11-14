Ops.money <- function(e1, e2) {
  v <- get(.Generic)(unclass(e1), unclass(e2))
  if (.Generic %in% c("+", "-", "*", "/")) structure(v, class = "money") else v
}
print.money <- function(x, ...) cat("$", unclass(x), "\n")
m <- structure(5, class = "money")
m + 1
2 * m
m > 3
"-.temp" <- function(e1, e2) "minus"
structure(1, class = "temp") - 1
