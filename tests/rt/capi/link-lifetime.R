# Stack links (docs/g3.md §4, crates/robigo-rt/src/links.rs): GNU R links
# the node stack's values at a complex assignment (STARTASSIGN, eval.c:8209)
# and at the compiler's INCLNK (eval.c:8691), and drops them when the
# committing activation, loop or INCLNKSTK region ends (eval.c:7211-7214,
# 7817, 8709); robigo-rt links its activations' registers for the length
# of each assignment. The difference reaches R only through reference
# counts (refcnt(), inspect(), tracemem(), address(); robigo-rt has none of
# them yet) and C only through MAYBE_SHARED/REFCNT/NAMED (robigo-rt's C API
# exports none) or through a copy at a write. This case checks the writes:
# each prints whether `v[i] <- x` wrote in place (TRUE) by comparing the
# object's address before and after, after the shapes that leave the value
# in a register or on GNU R's stack.
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
addr <- function(v) .Call("rt_addr", v)
# 1. the value read for an index, an arithmetic operand, a builtin's
#    argument, then written
f1 <- function() {
  v <- c(1, 2, 3)
  w <- v[1] + v[2]
  s <- sum(v)
  n <- length(v)
  a <- addr(v)
  v[3] <- 9
  c(addr(v) == a, w, s, n)
}
print(f1()); print(f1())
# 2. two writes in one activation: the first one's links held in GNU R
f2 <- function() {
  v <- c(1, 2, 3); u <- c(4, 5)
  u[1] <- 0
  a <- addr(v)
  v[1] <- 9
  addr(v) == a
}
print(f2()); print(f2())
# 3. in a loop, the write after other writes
f3 <- function() {
  v <- c(1, 2, 3); u <- numeric(3); ok <- logical(3)
  for (i in 1:3) {
    u[i] <- i
    a <- addr(v)
    v[i] <- i * 10
    ok[i] <- addr(v) == a
  }
  ok
}
print(f3()); print(f3())
# 4. the loop's own sequence is written in the loop: copied (for keeps its
#    sequence), and the loop still runs over the old values
f4 <- function() {
  v <- c(1, 2, 3); seen <- numeric(0)
  for (e in v) { v[2] <- 99; seen <- c(seen, e) }
  list(v, seen)
}
print(f4()); print(f4())
# 5. a caller's pending operand: the callee's write of the same global
#    copies, and the caller adds the old value
x <- c(1, 2, 3)
g5 <- function() { u <- c(1, 2); u[1] <- 5; a <- addr(x); x[1] <<- 99; c(addr(x) == a, 0) }
h5 <- function() x + g5()[2]
print(h5()); print(x)
# 6. after the callee that wrote has returned, the caller writes
x <- c(1, 2, 3)
k6 <- function() { u <- c(1, 2); u[1] <- 5; 0 }
h6 <- function() { v <- c(1, 2, 3); k6(); a <- addr(v); v[1] <- 7; addr(v) == a }
print(h6()); print(h6())
