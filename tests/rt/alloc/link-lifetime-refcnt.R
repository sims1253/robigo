# GNU R's stack links seen through reference counts (the recording is the
# target for robigo-rt's .Internal(refcnt()), which is not implemented yet:
# crates/robigo-rt/src/links.rs, docs/ledger/g3/2026-11-18-x9-4b-link-lifetime.md).
# A value pending as a caller's operand across a call (`x + g()`) holds the
# compiler's INCLNK link (eval.c:8691) for the whole callee, and links
# committed in an activation are held until it ends (eval.c:7211-7214).
# robigo-rt links its registers only for the length of each assignment, so
# its counts read 1 where GNU R's read 2 below.
r <- new.env()
note <- function(tag) assign(tag, .Internal(refcnt(get("x", globalenv()))), envir = r)
x <- c(1, 2, 3)

# 1. before, after one assignment, after a second, then in a loop
g1 <- function() {
    note("1.before")
    y <- c(1, 2); y[1] <- 5
    note("1.after-first")
    y[2] <- 6
    note("1.after-second")
    for (i in 1:2) { z <- i; note(paste0("1.loop", i)) }
    0
}
h1 <- function() x + g1()
invisible(h1())
note("1.after-return")

# 2. the assignment in a callee that has returned
k2 <- function() { y <- c(1, 2); y[1] <- 5; 0 }
g2 <- function() { note("2.before"); k2(); note("2.after-callee"); 0 }
h2 <- function() x + g2()
invisible(h2())

# 3. no pending operand: the callee is called on its own
g3 <- function() { note("3.before"); y <- c(1, 2); y[1] <- 5; note("3.after"); 0 }
invisible(g3())

for (n in sort(ls(r))) cat(n, get(n, envir = r), "\n")
