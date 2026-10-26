# Calls of pure closures run inline must behave like real calls
a <- function(i, j) 1 / ((i + j) * (i + j + 1) / 2 + i + 1)
g <- function(n) { s <- 0; for (i in 1:n) s <- s + a(i, 3); s }
print(g(100), digits = 17)
sq <- function(x) x * x
sq(4)
(function() sq(3))()
k <- 1
addk <- function(x) x + k
for (i in 1:3) { k <- i * 10; print(addk(i)) }
outer <- function() {
  k <- 1
  addk2 <- function(x) x + k
  r <- 0
  for (i in 1:3) { k <- i; r <- r + addk2(i) }
  r
}
outer()
sgn <- function(x) if (x > 0) 1 else if (x < 0) -1 else 0
h <- function() { r <- numeric(0); for (v in c(-2, 0, 3)) r <- c(r, sgn(v)); r }
h()
tryCatch({ x <- NA; sgn(x) }, error = function(e) print(conditionCall(e)))
ov <- function(x) x + 1L
loop_ov <- function() { r <- 0L; for (i in 1:2) r <- ov(.Machine$integer.max - 1L + i - 1L); r }
loop_ov()
sq_orig <- sq
redefine_callee <- function() {
  r <- numeric(0)
  for (i in 1:3) {
    r <- c(r, sq(i))
    if (i == 2) sq <<- function(x) -x
  }
  r
}
redefine_callee()
sq <- sq_orig
sq(5)
lazy <- function(x) sq(x)
lazy(6)
shadow <- function() {
  r <- numeric(0)
  for (i in 1:2) {
    r <- c(r, sq(i))
    sq <- 99
  }
  r
}
shadow()
bad <- function(x) x + "a"
caller <- function() { for (i in 1:2) bad(i) }
res <- tryCatch(caller(), error = function(e) conditionMessage(e))
res
defaults <- function(x, y = 2) x * y
dd <- function() { r <- 0; for (i in 1:3) r <- r + defaults(i); r }
dd()
dots <- function(...) 1
dd2 <- function() { r <- 0; for (i in 1:3) r <- r + dots(i); r }
dd2()
fact <- function(n) if (n <= 1) 1 else n * fact(n - 1)
fact(10)
vis <- function(x) x * 2
vis(21)
nan <- function(x) sqrt(x)
nn <- function() { for (v in c(4, -1)) print(nan(v)) }
nn()
withc <- function(x) c(x, x)
wc <- function() { r <- list(); for (i in 1:2) r[[i]] <- withc(i); r }
wc()
