# Frame recycling (D5, docs/g3.md §5.3): a closure frame that something can
# still reach when its call returns is never reused. Each way a frame can
# escape is taken here, in compiled loops: the frame is kept, other calls
# that would take a recycled frame of its size run, then the kept frame is
# read. Under ROBIGO_FRAME_QUARANTINE=1 a wrongly recycled frame fails
# where it is read. Each escape here is a count or the call's own value; no
# path in robigo-rt's base reaches the pool with only an uncounted
# reference, which the ESCAPED bit is for (switching the bit off changes
# nothing here, as in docs/ledger/g3/2026-11-14-d5-runs.md).
churn <- function(n) { s <- 0; for (i in 1:n) s <- s + length(list(i, i)); s }
# environment() as the value
mk <- function(v) { x <- v; environment() }
for (k in 1:3) { es <- list(); for (i in 1:4) es[[i]] <- mk(i * 10); churn(20) }
print(c(get("x", es[[1]]), get("x", es[[4]])))
# a closure made in the frame (its enclosure counts the frame)
adder <- function(n) { m <- n; function(x) x + m }
for (k in 1:3) { a1 <- adder(1); a2 <- adder(100); churn(20) }
print(c(a1(1), a2(1)))
# a promise of the frame, forced after the call returned
lazy <- function(x) function() x
outer <- function(v) { w <- v * 2; lazy(w + 1) }
for (k in 1:3) { f1 <- outer(1); f2 <- outer(5); churn(20) }
print(c(f2(), f1()))
# parent.frame(): the caller's frame, read through a context
pf <- function() parent.frame()
caller <- function(v) { y <- v; pf() }
for (k in 1:3) { e1 <- caller(7); churn(20) }
print(get("y", e1))
# environment() kept by on.exit code and by <<-
ox <- function(v) { z <- v; on.exit(kept_exit <<- environment()); z }
sa <- function(v) { z <- v; kept_super <<- environment(); z }
for (k in 1:3) { ox(3); sa(4); churn(20) }
print(c(get("z", kept_exit), get("z", kept_super)))
# assign() into the global environment, and a list holding the frame
as <- function(v) { q <- v; assign("kept_assign", environment(), envir = globalenv()); invisible(NULL) }
li <- function(v) { q <- v; list(environment()) }
for (k in 1:3) { as(8); l <- li(9); churn(20) }
print(c(get("q", kept_assign), get("q", l[[1]])))
# a new environment whose enclosure is the frame
ne <- function(v) { p <- v; new.env(parent = environment()) }
for (k in 1:3) { n1 <- ne(11); churn(20) }
print(get("p", n1))
# an attribute holding the frame
at <- function(v) { r <- v; structure(1, env = environment()) }
for (k in 1:3) { s1 <- at(12); churn(20) }
print(get("r", attr(s1, "env")))
# an S3 method's frame returned through UseMethod
gen <- function(x) UseMethod("gen")
gen.foo <- function(x) { u <- unclass(x); environment() }
for (k in 1:3) { m1 <- gen(structure(13, class = "foo")); churn(20) }
print(get("u", m1))
# frames that do not escape are reused: recursion through real calls keeps
# every level's locals
rec <- function(n) { if (n == 0) return(0); v <- list(n); rec(n - 1) + v[[1]] }
for (k in 1:3) r <- rec(30)
print(r)
