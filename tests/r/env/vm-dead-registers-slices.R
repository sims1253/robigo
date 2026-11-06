# Slice stores in effect position (`x[i, , k] <- v`, `x[i, , k] <<- v`,
# vm.rs Op::SetIndexN / Op::SuperSetIndexN) copy the elements, so the
# value is dead afterwards: an environment it carries in an attribute is
# finalized by the next gc(), as GNU R's bytecode pops it (#357). Closure
# bodies, so the VM and the JIT run them; counters, not prints.
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
run("slice store", function() {
  m <- matrix(0, 2, 2)
  m[, 1] <- structure(c(1, 2), fin = mk())
  invisible(gc()); c(n$k, m[2, 1])
})
run("slice store in a loop", function() {
  d <- array(0, c(2, 2, 3))
  for (i in 1:2) d[i, 2, ] <- structure(c(1, 2, 3) * i, fin = mk())
  invisible(gc()); c(n$k, d[2, 2, 3])
})
run("slice superassignment", function() {
  g <- matrix(0, 2, 2)
  h <- function() g[1, ] <<- structure(c(5, 6), fin = mk())
  h(); invisible(gc()); c(n$k, g[1, 2])
})
run("slice store's value used", function() {
  m <- matrix(0, 2, 2)
  v <- (m[, 2] <- structure(c(3, 4), fin = mk()))
  invisible(gc()); c(n$k, is.environment(attr(v, "fin")))
})
