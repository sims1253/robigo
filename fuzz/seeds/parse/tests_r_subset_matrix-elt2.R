# x[[i, j]] and x[[i, j]] <- v in loops (the VM's matrix element ops).
x <- matrix(nrow = 3, ncol = 2)
for (i in 1:3) for (j in 1:2) x[[i, j]] <- i * 10 + j
print(x)
s <- 0
for (i in 1:3) for (j in 1:2) s <- s + x[[i, j]]
print(s)
y <- x
y[[2, 2]] <- 7L
print(x[[2, 2]])
print(y[[2, 2]])
l <- matrix(list(1, "a", TRUE, NULL), 2)
l[[1, 2]] <- list(9)
print(l[[1, 2]])
print(l[[2, 2]])
m <- matrix(letters[1:4], 2, dimnames = list(c("r1", "r2"), c("c1", "c2")))
print(m[["r1", "c2"]])
print(m[[2, 1]])
m[[2, 1]] <- "z"
print(m)
tryCatch(x[[4, 1]], error = function(e) print(conditionMessage(e)))
tryCatch(x[[4, 1]] <- 1, error = function(e) print(conditionMessage(e)))
tryCatch(x[[1, 1]] <- 1:2, error = function(e) print(conditionMessage(e)))
tryCatch(x[[NA, 1]], error = function(e) print(conditionMessage(e)))
f <- function(a) {
  for (k in 1:2) a[[k, 1]] <- 99
  a
}
print(f(x))
print(x)
print(x[[1.9, 2]])
z <- structure(matrix(1:4, 2), class = "foo")
`[[.foo` <- function(x, i, j) "dispatched"
print(z[[1, 1]])
