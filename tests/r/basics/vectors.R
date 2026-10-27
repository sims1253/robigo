x <- c(10, 20, 30, 40, 50)
x
length(x)
x[2]
x[c(1, 3)]
x[-1]
x[x > 25]
x[10]
x[c(TRUE, FALSE)]
rev(x)
sum(x)
mean(x)
max(x)
min(x)
range(x)
prod(1:6)
cumsum(1:10)
seq(1, 10, by = 2)
seq(0, 1, length.out = 5)
seq_len(5)
seq_along(c("a", "b", "c"))
rep(1:3, times = 2)
rep(1:3, each = 2)
rep("x", 3)
c(1, "a", TRUE)
c(1, TRUE, NA)
c(list(1), 2)
y <- 1:10
y[y %% 2 == 0]
which(y > 7)
any(y > 9)
all(y > 0)
z <- c(b = 2, a = 1)
z["a"]
names(z)
sort(c(3, 1, 2))
order(c(3, 1, 2))
unique(c(1, 2, 2, 3, 1))
length(NULL)
