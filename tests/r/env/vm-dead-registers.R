# A value compiled code no longer needs must not outlive a gc(): GNU R's
# bytecode pops it off the node stack (POP after each non-last `{`
# statement and after a loop body, ENDFOR for the sequence, SETVAR's value
# in effect position), and memory.c scans the stack only up to
# R_BCNodeStackTop. Values still in use must survive, though. Closure
# bodies, so the VM and the JIT run them; counters, not prints, so the
# output cannot shift between tiers (#357).
n <- new.env(parent = globalenv())
n$k <- 0L
mk <- function() {
  e <- new.env(parent = globalenv())
  reg.finalizer(e, function(x) n$k <- n$k + 1L)
  e
}
run <- function(name, f) {
  n$k <- 0L
  r <- f()
  invisible(gc())
  cat(name, ": during ", paste(r, collapse = " "), ", after ", n$k, "\n", sep = "")
}

# Dead values (#357's cases).
run("previous statement's value", function() { local(o <- mk()); invisible(gc()); n$k })
run("call temporary", function() { length(list(mk())); invisible(gc()); n$k })
run("loop body's last value", function() { for (i in 1:2) local(o <- mk()); invisible(gc()); n$k })
run("if branch value", function() { if (TRUE) local(o <- mk()); invisible(gc()); n$k })
run("bare call", function() { mk(); invisible(gc()); n$k })
run("bare variable", function() { o <- mk(); o; rm(o); invisible(gc()); n$k })
run("while body", function() { i <- 0; while (i < 2) { i <- i + 1; local(o <- mk()) }; invisible(gc()); n$k })
run("repeat body", function() { repeat { local(o <- mk()); break }; invisible(gc()); n$k })

# Stores in effect position keep the value in the binding only.
run("effect store", function() { x <- mk(); x <- NULL; invisible(gc()); n$k })
run("effect store in a loop", function() { for (i in 1:3) x <- mk(); x <- NULL; invisible(gc()); n$k })
run("effect index store", function() { l <- list(1); l[[1]] <- mk(); l[[1]] <- 1; invisible(gc()); n$k })
run("effect index store [", function() { l <- list(1); l[1] <- list(mk()); l[1] <- list(1); invisible(gc()); n$k })
run("effect dollar store", function() { l <- list(a = 1); l$a <- mk(); l$a <- 1; invisible(gc()); n$k })
run("effect complex store", function() { l <- list(a = list(1)); l$a[[1]] <- mk(); l$a[[1]] <- 1; invisible(gc()); n$k })
run("effect superassignment", function() {
  g <- NULL
  h <- function() { g <<- mk(); g <<- NULL }
  h()
  invisible(gc())
  n$k
})

# Values still in use must not be collected.
run("for sequence during the loop", function() {
  k <- integer()
  for (x in list(mk(), 1)) { invisible(gc()); k <- c(k, n$k) }
  invisible(gc())
  c(k, n$k)
})
run("for sequence after a break", function() {
  for (x in list(1, mk())) break
  invisible(gc())
  n$k
})
run("value returned past on.exit(gc())", function() {
  f <- function() { on.exit(invisible(gc())); mk() }
  r <- f()
  k <- n$k
  rm(r)
  invisible(gc())
  c(k, n$k)
})
run("argument while a later one runs gc()", function() {
  r <- c(mk(), invisible(gc()))
  k <- n$k
  rm(r)
  invisible(gc())
  c(k, n$k)
})
run("value of an assignment", function() {
  y <- (x <- mk())
  rm(x)
  invisible(gc())
  k <- n$k
  rm(y)
  invisible(gc())
  c(k, n$k)
})
run("last value is the result", function() {
  f <- function() { o <- mk(); o }
  r <- f()
  invisible(gc())
  k <- n$k
  rm(r)
  invisible(gc())
  c(k, n$k)
})
