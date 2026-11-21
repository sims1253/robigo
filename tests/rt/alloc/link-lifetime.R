# Stack links' lifetime (crates/robigo-rt/src/links.rs;
# docs/ledger/g3/2026-11-18-x9-4b-link-lifetime.md). GNU R 4.6.2 links the
# byte-code node stack's values at a complex assignment (STARTASSIGN,
# eval.c:8209, INCLNK_stack_commit) and drops the links when the committing
# activation ends (eval.c:7211-7214); robigo-rt drops them when the
# assignment ends. A builtin that writes its argument in place unless it is
# shared, comment<- (attrib.c:525, MAYBE_SHARED), shows the difference on a
# global still pending as a caller's operand (`x + g()`), after the callee's
# own complex assignment. Each shape runs once first, so that the JIT has
# compiled every closure before it is measured (a compile inside the call
# makes its own assignments, which link the pending operand too).
setc <- function() .Internal(`comment<-`(get("x", globalenv()), "changed"))
h <- function() { for (i in 1L) NULL; x + g() }
shape <- function(label, gf) {
  g <<- gf
  x <<- c(1, 2, 3); invisible(h())
  x <<- c(1, 2, 3); invisible(h())
  cat(label, ":", if (is.null(comment(x))) "copied" else "in place", "\n")
}
# no pending caller: in place
g0 <- function() { for (i in 1L) NULL; u <- c(1, 2); u[1] <- 5; setc(); 0 }
x <- c(1, 2, 3); g0(); x <- c(1, 2, 3); g0(); cat("no caller :", if (is.null(comment(x))) "copied" else "in place", "\n")
shape("pending, no assignment", function() { for (i in 1L) NULL; setc(); 0 })
shape("pending, assignment before", function() { for (i in 1L) NULL; u <- c(1, 2); u[1] <- 5; setc(); 0 })
shape("pending, assignment after", function() { for (i in 1L) NULL; setc(); u <- c(1, 2); u[1] <- 5; 0 })
k <- function() { for (i in 1L) NULL; u <- c(1, 2); u[1] <- 5; 0 }
shape("pending, assignment in a returned callee", function() { for (i in 1L) NULL; k(); setc(); 0 })
shape("pending, assignment before, in a loop", function() { for (i in 1:2) { u <- c(1, 2); u[i] <- 5 }; setc(); 0 })
