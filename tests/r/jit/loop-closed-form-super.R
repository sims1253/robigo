# Loops that count with `<<-` (native since the loop JIT runs `<<-`) in
# closed form: the enclosing binding ends as running every iteration
# leaves it, including where the closed form declines.
counter <- function(n) { x <- 0; f <- function() { while (x < n) x <<- x + 1 }; f(); x }
for (r in 1:3) print(counter(100000))
for (r in 1:3) print(counter(10.5))
for (r in 1:3) print(counter(-3))
for (r in 1:3) print(tryCatch(counter(NA), error = function(e) conditionMessage(e)))
g <- 0
glob <- function(n) { for (i in 1:n) g <<- i; invisible() }
for (r in 1:3) { glob(500); print(g) }
h <- function(n) { y <- 2147483600L; k <- function() { i <- 0L; while (i < n) { y <<- y + 1L; i <- i + 1L } }; k(); y }
for (r in 1:3) print(h(40L))
for (r in 1:3) print(h(100L))
w <- function(n) { s <- 0; last <- 0; f <- function() { for (i in 1:n) { last <<- s; s <<- s + 2 } }; f(); c(s, last) }
for (r in 1:3) print(w(1000))
for (r in 1:3) print(w(1))
