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
# `s` steps into closures a builtin calls (lapply's FUN), as R_execClosure
# checks the caller's frame.
inner <- function(x) x + 1
outer <- function() { browser(); lapply(1:2, inner) }
outer()
s
s
s
s
s
s
s
s
s
s
s
c
# `break` and `next` typed at a loop's step prompt act on that loop.
fb <- function() { browser(); for (i in 1:3) cat(i); cat("after\n") }
fb()
n
n
break
c
fb()
n
n
next
c
c
wb <- function() { browser(); i <- 0; while (i < 3) i <- i + 1; cat("i =", i, "\n") }
wb()
n
n
n
break
c
wb()
n
n
n
next
c
c
# ... and at the "(while)" prompt; a `next` in the body skips that prompt.
wb()
n
n
n
n
next
n
c
c
wn <- function() {
  browser()
  i <- 0
  while (i < 3) { i <- i + 1; if (i == 2) next; cat(i, "") }
  cat("\n")
}
wn()
n
n
n
n
n
n
n
n
n
n
n
break
c
cat("end\n")
