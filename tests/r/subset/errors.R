# Error and warning messages of subsetting and subassignment.
err <- function(expr) tryCatch(expr, error = function(e) conditionMessage(e))
call_of <- function(expr) tryCatch(expr, error = function(e) conditionCall(e))
x <- c(a = 1, b = 2, c = 3)
err(x[[5]])
err(x[["zz"]])
err(x[[0]])
err(x[[-1]])
err(x[[c(1, 2)]])
err(x[[integer(0)]])
err(x[[NA]])
err(x[list(1)])
err(x[[list(1)]])
err(x[c(-1, 2)])
err(x[c(-1, NA)])
err(x[1, 2])
err(x$a)
err(x[[]])
y <- 1:2
err(y[[-1]])
err(y[[-3]])
l <- list(p = 1, q = list(r = 2))
err(l[[3]])
err(l[[c(2, 5)]])
err(l[[c(1, 1, 1)]])
err(l[[c("q", "zz")]])
err(l[[c("q", "zz", "w")]])
m <- 1:4
dim(m) <- c(2L, 2L)
err(m[3, 1])
err(m[1, 3])
err(m[[3, 1]])
err(m[1, 2, 3])
err(m[[1, 2, 3]])
err(m["a", 1])
err(m[, 1, 1])
err(m[-1, NA])
idx <- c(1L, -1L, 1L, 1L)
dim(idx) <- c(2L, 2L)
err(m[idx])
idx3 <- 1:6
dim(idx3) <- c(2L, 3L)
err(m[idx3])
f <- function() NULL
err(f[1])
err(f[[1]])
err(f$a)
err(new.env()[1])
err(quote(a)[1])
# replacement errors leave the target unchanged
z <- 1:3
err(z[[5]] <- 1:2)
z
err(z[[2]] <- integer(0))
z
err(z[[]] <- 1L)
err(z[1:2] <- integer(0))
z
err(z[c(NA, 1)] <- 1:2)
z
err(z[list(1)] <- 1L)
z
err(z[["a", "b"]] <- 1L)
err(z[1, 1] <- 1L)
z
err(m[3, 1] <- 1L)
err(m[1:2, 1] <- 1:3)
err(m[, 1, 1] <- 1L)
m
r <- as.raw(1:2)
err(r[1] <- TRUE)
r
lst <- list(1)
err(lst[[c(2, 1)]] <- 1)
err(lst[[c(1, 1, 1)]] <- 1)
lst
err(f[1] <- 2)
err(f[[1]] <- 2)
err(f$a <- 2)
f
q <- quote(g(x))
err(q[[5]] <- NULL)
err(q[-(1:2)] <- NULL)
q
call_of(z[[5]] <- 1:2)
call_of(z[[5]])
call_of(m[3, 1] <- 1L)
g <- function(v) { v[[10]] <- 1:2; v }
call_of(g(1:3))
# warnings
w <- 1:5
withCallingHandlers(
  w[1:3] <- 1:2,
  warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }
)
w
v <- 1:3
v$a <- 9
v
pm <- list(alpha = 1)
pm[["al", exact = NA]]
pm$al
