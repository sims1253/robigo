x <- 1:2
attr(x, "names") <- structure(c(1, 2), names = c("q", "r"), abc = 3)
dput(names(x))
attr(x, "names") <- structure(1, names = "q", abc = 3, class = "labels")
dput(names(x))
attr(x, "names") <- structure(c("a", "b"), class = "labels", extra = TRUE)
dput(names(x))
attr(x, "names") <- factor(c("b", "a"))
dput(names(x))
y <- x
names(y) <- c("new", "names")
dput(names(x))
dput(names(y))
