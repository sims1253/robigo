# A closure's scalar value returned to its compiled caller (robigo hands it
# over unboxed): by return() and as the body's last value, double, integer
# and logical, with on.exit code (which may read the value, so it is
# boxed there), and a getter call of a complex assignment whose getter
# returns a scalar. Each runs three times in a compiled loop: the first
# calls compile the closures.
r_dbl <- function() return(1.5)
r_int <- function() return(2L)
r_lgl <- function() return(TRUE)
v_dbl <- function(x) x * 2
v_int <- function(x) x + 1L
v_lgl <- function(x) x > 1
ex <- function(x) { on.exit(if (k == 3) cat("exit", x, "\n")); return(x * 10L) }
ex2 <- function(x) { on.exit(if (k == 3) cat("exit2\n")); x + 0.5 }
for (k in 1:3) {
  r <- list(r_dbl(), r_int(), r_lgl(), v_dbl(k), v_int(k), v_lgl(k), ex(k), ex2(k))
}
print(r)
print(vapply(r, typeof, ""))
gt <- function(x) 5
`gt<-` <- function(x, value) value + x
run <- function() {
  z <- 1
  gt(z)[1] <- 7
  z
}
for (k in 1:3) r <- run()
print(r)
s <- 0
for (k in 1:3) s <- s + r_dbl() + v_int(k) + ex2(k)
print(s)
