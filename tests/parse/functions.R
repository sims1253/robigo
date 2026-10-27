function(x, y = 2, ...) x + y
function() NULL
\(x) x + 1
f <- function(a,
              b = c(1, 2),
              ...) {
  a + b
}
g <- \(x, y) x |> h(y)
compose <- function(f, g) function(x) g(f(x))
function(x) x = 3
function(x = function(y) y) x
lapply(xs, function(x) x * 2)
sapply(xs, \(x) x^2)
do.call("rbind", lapply(l, function(z) z))
function(`odd name`, .x, ..., ..1) NULL
