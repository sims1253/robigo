# x[[i]] <- v in loops long enough for native code, inside functions (the
# edge cases come after the loop is compiled).
f1 <- function() { x <- vector(length = 40); for (i in 1:40) x[[i]] <- i; x }
print(f1())
f2 <- function() { y <- numeric(30); for (i in 1:40) y[[i]] <- i / 2; y }
print(f2())
f3 <- function() { z <- integer(40); for (i in 1:40) z[[i]] <- if (i == 35) 2.5 else i; z }
print(f3())
f4 <- function() { w <- 1:40; for (i in 1:40) w[[i]] <- if (i %% 7 == 0) NA_integer_ else -i; w }
print(f4())
f5 <- function() {
  nm <- seq(1, 40)
  names(nm) <- paste0("n", 1:40)
  for (i in 1:40) nm[[i]] <- i * 3
  nm[c(1, 20, 40)]
}
print(f5())
f6 <- function() { l <- as.list(1:40); for (i in 1:40) l[[i]] <- i * 2; unlist(l)[c(1, 40)] }
print(f6())
f7 <- function() {
  q <- numeric(40)
  r <- tryCatch({ for (i in c(1:30, 0)) q[[i]] <- 9 ; "done" }, error = function(e) conditionMessage(e))
  c(r, sum(q))
}
print(f7())
f8 <- function() { d <- double(40); for (i in 1:40) d[[i]] <- TRUE; sum(d) }
print(f8())
f9 <- function(n) { v <- double(n); for (i in seq_len(n)) v[[i]] <- i * 0.25; sum(v) }
print(f9(100))
