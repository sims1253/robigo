# Semantics the bytecode compiler must preserve.
f <- function() {
  `+` <- function(a, b) a * b
  2 + 3
}
f()
g <- function(n) {
  s <- 0
  for (i in seq_len(n)) {
    if (i %% 2 == 0) next
    if (i > 7) break
    s <- s + i
  }
  c(s, i)
}
g(10)
g(0)
h <- function() {
  for (i in 1:5) if (i == 3) return(i * 10)
  -1
}
h()
`+` <- function(e1, e2) "plus redefined"
1 + 1
k <- function() 2 + 2
k()
rm(`+`)
1 + 1
k()
loopvar <- function() {
  for (x in c(1.5, 2.5)) NULL
  x
}
loopvar()
emptyloop <- function() {
  for (x in integer(0)) NULL
  x
}
emptyloop()
w <- function() {
  i <- 0L
  while (TRUE) {
    i <- i + 1L
    if (i >= 5L) break
  }
  repeat {
    i <- i - 2L
    if (i < 0L) break
  }
  i
}
w()
inv <- function() x <- 5
inv()
(inv())
v <- inv()
v
noelse <- function() if (FALSE) 1
noelse()
is.null(noelse())
cond <- function(x) if (x) "yes" else "no"
cond(TRUE)
r <- tryCatch(cond(NA), error = function(e) conditionMessage(e))
r
big <- function() {
  x <- .Machine$integer.max
  x + 1L
}
big()
mixed <- function() {
  a <- 1L
  b <- 2.5
  c(a + b, a * 2L, a / 2L, 7L %/% 2L, 7L %% 3L, -a, !TRUE, 2^10, 5 %% -3)
}
mixed()
lst <- function() {
  out <- character(0)
  for (el in list(1, "a", TRUE)) out <- c(out, class(el))
  out
}
lst()
fac <- function() {
  out <- character(0)
  for (lv in factor(c("b", "a"))) out <- c(out, lv)
  out
}
fac()
nested <- function() {
  total <- 0
  for (i in 1:3) for (j in 1:3) {
    if (j > i) next
    total <- total + i * j
  }
  total
}
nested()
shadow_if <- function() {
  `if` <- function(cond, yes, no) "custom if"
  if (TRUE) 1 else 2
}
shadow_if()
cmp <- function(a, b) c(a < b, a == b, a >= b, NA > 1)
cmp(1, 2)
cmp(2L, 2)
# forcing a promise keeps the visibility of its expression
pass <- function(expr) expr
pass(invisible(1))
pass(2)
loop_pass <- function(expr) { for (i in 1:2) NULL; expr }
loop_pass(invisible(3))
withVisible(loop_pass(invisible(4)))$visible
