do.call("paste", list("a", "b", sep = "-"))
do.call(sum, list(1, 2, 3))
f <- function(x, y) x - y
do.call(f, list(y = 1, x = 10))
do.call("c", list(a = 1, b = 2))
