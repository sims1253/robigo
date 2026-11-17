# Factors of strings made by paste and sprintf: their levels are found by
# unique and matched by match, which GNU R hashes by pointer.
f <- factor(c(paste0("x", c(1, 2, 1)), "x3", "x2"))
print(levels(f)); print(as.integer(f))
g <- factor(sprintf("l%d", c(2, 1, 2)), levels = paste0("l", 1:2))
print(as.integer(g))
h <- factor(as.character(c(3, 1, 3)), levels = c("1", "3"))
print(h); print(as.integer(h))
x <- factor(c("a", "b"))
levels(x) <- paste0(c("a", "b"), "2")
print(x)
