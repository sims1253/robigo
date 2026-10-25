# Assorted subsetting and subassignment edge cases.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
x <- c(a = 1, b = 2, c = 3)
x[c(z = 1)]
x[-c(1, 1)]
x[c(1, 1)] <- c(5, 6)
x
err(x[as.raw(1)])
err(x[1i])
err(x[[1i]])
err(x[[as.raw(1)]])
l <- list(a = 1)
l[["zz"]] <- NULL
l
l$a$b <- 1
l
l2 <- list()
l2$a$b <- 1
l2
l2[["a"]]$c <- 2
l2
attr(l2$a, "x") <- "attr"
attributes(l2$a)
fi <- c(2L, 1L)
attr(fi, "levels") <- c("u", "v")
class(fi) <- "factor"
list(10, 20)[[fi[1]]]
big <- 1:2000
names(big) <- as.character(big)
big[c("5", "1999", "zz")]
big[c("5", "new")] <- 0L
length(big)
big[2000:2001]
v <- 1:3
is.na(v) <- 2
v
err({x2 <- 1:3; x2[[1, exact = FALSE]] <- 1})
nested <- list(1, list(2, 3))
nested[[2]][[1]] <- "two"
nested[[c(2, 2)]] <- "three"
nested
nm <- c(one = 1)
nm[["two"]] <- 2
nm
names(nm)
x3 <- 1:3
x3[5] <- 2.5
x3
x4 <- c(a = 1)
x4[c("b", "c")] <- list(2, "c")
x4
y <- 1:10
y[-(1:3)] <- 0L
y
y[y > 100] <- 1L
y
z <- c(1, 2)
z[c(-1, -2)] <- 9
z
emp <- list()
emp[[1]] <- NULL
emp
emp[1] <- list(NULL)
emp
ch <- character(0)
ch[["a"]] <- "A"
ch
nl <- NULL
nl[[1]] <- 1:3
nl
nl2 <- NULL
nl2[[2]] <- "x"
nl2
nl3 <- NULL
nl3$a <- NULL
nl3
q <- quote(f(a, b))
q[[4]] <- quote(c)
q
q[["named"]] <- 1
q
q$named <- NULL
q
q[-1]
typeof(q[-1])
q[2:3] <- list(1, 2)
q
pl <- pairlist(1, 2)
pl[[3]] <- 3
pl
typeof(pl)
pl[[1]] <- NULL
pl
r <- as.raw(1:3)
r[[2]]
r[-1]
lg <- c(TRUE, FALSE)
lg[[3]] <- NA
lg
s <- "a"
s[[3]] <- "c"
s
