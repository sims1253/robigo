f <- factor(c("low", "high", "medium", "low"))
f
levels(f)
nlevels(f)
as.integer(f)
table(f)
f2 <- factor(c("a", "b"), levels = c("b", "a"))
f2
as.character(f2)
summary(factor(c("x", "y", "x")))
droplevels(factor(c("a", "b"))[1])
factor(c(1, 2, 3), labels = c("one", "two", "three"))
