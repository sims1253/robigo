# Not interactive: browser() reads its commands from the script.
f <- function(x) {
  y <- x + 1
  browser()
  y * 2
}
f(1)
y
x + 10
invisible(7)
c
cat("after f\n")
# where
g <- function() f(5)
g()
where
c
cat("after g\n")
# return() at the prompt is browser()'s value
h <- function() { v <- browser(); cat("v is", v, "\n"); 3 }
h()
return(42)
c
# An error at the prompt is reported and the browser goes on.
k <- function() { browser(); "k done" }
k()
stop("oops")
undefined_variable
c
# n steps through the rest of the function.
s1 <- function(a) {
  browser()
  b <- a * 2
  if (b > 1) b <- b + 1
  b
}
s1(3)
n
n
b
n
n
n
cat("stepped\n")
# browser() at top level
browser()
1 + 1
c
# browser(expr = FALSE) does nothing; browserText() and friends
browser(expr = FALSE)
t1 <- function() browser("my text", condition = "cond")
t1()
browserText()
browserCondition()
c
# An empty line leaves the browser; the frame is still being debugged.
e1 <- function() {
  browser()
  cat("one\n")
  cat("two\n")
}
e1()

c
cat("done\n")
