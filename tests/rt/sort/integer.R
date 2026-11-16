# sort() of integers and logicals: decreasing, na.last, the methods,
# names and index.return
x <- c(5L, NA, 3L, -2L, 3L, 0L, NA, 9L, -2L, 1L)
print(sort(x))
print(sort(x, decreasing = TRUE))
print(sort(x, na.last = TRUE))
print(sort(x, na.last = FALSE, decreasing = TRUE))
for (m in c("shell", "quick", "radix")) {
  print(sort(x, method = m))
  print(sort(x, method = m, decreasing = TRUE, na.last = TRUE))
}
names(x) <- letters[1:10]
print(sort(x))
print(sort(x, decreasing = TRUE, method = "shell"))
print(sort(x, method = "quick"))
y <- x[!is.na(x)]
print(sort(y, index.return = TRUE))
print(sort(y, index.return = TRUE, decreasing = TRUE, method = "shell"))
print(sort(unname(y), index.return = TRUE, method = "quick"))
print(sort(unname(y), partial = 3))
print(sort(unname(y), partial = c(2, 6)))
l <- c(TRUE, NA, FALSE, TRUE, FALSE)
print(sort(l))
print(sort(l, decreasing = TRUE, na.last = FALSE))
print(sort(integer()))
print(sort(NULL))
print(sort(c(3L, 1L, 2L), method = "radix", index.return = TRUE))
print(is.unsorted(x))
print(is.unsorted(c(1L, 2L, 2L)))
print(is.unsorted(c(1L, 2L, 2L), strictly = TRUE))
print(is.unsorted(c(1L, NA, 2L)))
print(is.unsorted(c(1L, NA, 2L), na.rm = TRUE))
print(.Internal(sort(c(3L, 1L, NA, 2L), FALSE)))
print(.Internal(sort(c(3L, 1L, NA, 2L), TRUE)))
