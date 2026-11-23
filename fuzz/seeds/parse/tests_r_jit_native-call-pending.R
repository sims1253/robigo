# A loop calling a closure compiled whole (after 512 calls) leaves its own
# pending assignments unwritten unless the closure reads one of those
# variables: a variable it reads (a cell), a function it calls, or a
# function it guards must still be seen with its new value.
f <- function(n) n + 1
g <- function(n) n + 100
body <- function() execute(nn)
execute <- f; nn <- 3
out <- numeric(0)
for (k in 1:1300) {
  if (k == 1100) execute <- g
  if (k == 1200) nn <- 50
  j <- k * 2
  r <- body()
  if (k %in% c(1099, 1100, 1199, 1200, 1300)) out <- c(out, r)
}
print(out); print(j)
`+` <- function(a, b) base::`+`(a, b) * 10
out2 <- numeric(0)
for (k in 1:3) out2 <- c(out2, body())
rm(`+`)
print(out2)
h <- function() { s <- 0; for (k in 1:1300) { nn <<- k; s <- s + body() }; s }
execute <- f
print(h())
w <- function() { tot <- 0; for (k in 1:1300) { v <- k; tot <- tot + body2(v) }; tot }
body2 <- function(a) a * 2
print(w())
