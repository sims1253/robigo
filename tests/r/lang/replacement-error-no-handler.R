# In-place replacements with no condition handler established: warnings
# are reported, and a failing replacement leaves the target as it was
# (#527). The error ends the script; options(error=) shows the target.
options(warn = 1)
x <- 1:3
x[1:2] <- 4:6
print(x)
y <- c(a = 1, b = 2)
names(y) <- 7
print(y)
options(warn = 0)
z <- "bad"
class(z) <- "integer"
print(z)
f <- function() {
  v <- c(1, 2, 3)
  v[1:2] <- 1:3
  v
}
print(f())

options(error = quote(print(x)))
x[[5]] <- 1:2
