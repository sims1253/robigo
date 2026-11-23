l <- list(1, "a", TRUE, NULL, c(1, 2))
l
length(l)
l
list(a = 1, b = list(c = 2))
structure(1:3, names = c("x", "y", "z"))
rapply(list(1, 2, list(3)), function(x) x * 10, how = "unlist")
emptyl <- list()
emptyl
length(emptyl)
is.list(emptyl)
x <- list(a = 1:3, b = letters[1:2])
x[["a"]][2]
rev(list(1, "b"))
