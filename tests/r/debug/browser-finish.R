# f finishes the innermost loop, then stepping resumes after it.
f1 <- function() {
  browser()
  for (i in 1:3) {
    cat("i =", i, "\n")
  }
  cat("after loop\n")
  "f1 done"
}
f1()
n
n
f
n
n
# f outside a loop finishes the function.
f2 <- function() {
  browser()
  x <- 1
  cat("x is", x, "\n")
  x + 1
}
f2()
n
f
cat("after f2\n")
# f inside a while body, with the (while) prompts suppressed
f3 <- function() {
  i <- 0
  browser()
  while (i < 3) {
    i <- i + 1
    cat("w", i, "\n")
  }
  i
}
f3()
n
n
n
f
n
# f in a debugged function that calls another debugged one
inner <- function(a) {
  b <- a + 1
  b * 2
}
outer <- function() {
  browser()
  r <- inner(1)
  r + 1
}
debug(inner)
outer()
n
f
n
n
n
cat("after outer\n")
undebug(inner)
# repeat
f4 <- function() {
  k <- 0
  browser()
  repeat {
    k <- k + 1
    if (k > 2) break
  }
  k
}
f4()
n
n
n
f
n
# nested loops: f finishes the inner one only. (`c` inside the inner
# loop is undone when each loop ends: do_for restores the frame's RDEBUG.)
f5 <- function() {
  browser()
  for (i in 1:2) {
    for (j in 1:2) cat(i, j, "\n")
    cat("outer", i, "\n")
  }
  "f5 done"
}
f5()
n
n
n
f
n
n
n
c
c
c
# browser() in a function lapply calls: f finishes that call
r <- lapply(1:2, function(i) {
  browser()
  cat("call", i, "\n")
  i * 10
})
f
n
f
unlist(r)
# f in a browser started at another browser's prompt finishes the
# function both run in; the outer browser keeps stepping until it is left.
f7 <- function() {
  browser()
  x <- 1
  cat("x", x, "\n")
  x + 1
}
f7()
browser()
f
{ cat("a\n"); cat("b\n") }
n
n
c
# f at top level marks the top level itself, which never ends: later
# browsers that inherit it do not step.
browser()
f
{ cat("a\n"); cat("b\n") }
g6 <- function() { browser(); cat("one\n"); cat("two\n") }
g6()
n
cat("end\n")
