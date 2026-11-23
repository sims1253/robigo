x <- 1:5
x[2] <- 10L
x
x[8] <- 1L
x
y <- c(a = 1, b = 2)
y["c"] <- 3
y
y[["a"]] <- 100
y
names(y)[2] <- "B"
y
l <- list(a = 1)
l$b <- "new"
l[["c"]] <- 1:3
l
l$a <- NULL
names(l)
m <- matrix(0, 2, 2)
m[1, 2] <- 5
m
m[2, ] <- c(7, 8)
m
v <- 1:10
v[v > 5] <- 0L
v
z <- c(1, 2, 3)
z[2] <- "two"
z
attr(z, "myattr") <- "hello"
attributes(z)
levels_test <- factor(c("a", "b", "a"))
levels(levels_test) <- c("X", "Y")
levels_test
lst <- list()
for (i in 1:3) lst[[i]] <- i^2
lst
dim(v) <- c(2, 5)
v
