# Loops in native code that assign with `<<-` (jit.rs, vm::jit_enter): the
# variable is the enclosing binding, written back when the loop ends or
# continues in the VM.
sw <- function(n) {
  x <- 0
  f <- function() { while (x < n) x <<- x + 1; x }
  r <- f()
  c(r, x)
}
print(sw(1000)); print(sw(0)); print(sw(10L))
cnt <- 0L
inc <- function(n) { for (i in 1:n) cnt <<- cnt + 1L; cnt }
print(inc(500)); print(cnt); print(typeof(cnt))
# leaving native code mid-loop: the value so far is in the binding
ov <- function() { k <- 2147483600L; g <- function() { for (i in 1:100) k <<- k + 1L; k }; r <- suppressWarnings(g()); c(r, k) }
print(ov())
na <- function() {
  s <- 0
  v <- c(1, 2, NA, 4)
  g <- function() { for (i in 1:200) { s <<- s + 1; if (i == 150) s <<- s + v[3] }; s }
  r <- g(); c(r, s)
}
print(na())
er <- function() {
  s <- 0
  g <- function() { for (i in 1:100) { s <<- s + 1; if (i == 50) stop("at 50") } }
  r <- tryCatch(g(), error = function(e) conditionMessage(e))
  list(r, s)
}
print(er())
br <- function() {
  s <- 0
  g <- function() { repeat { s <<- s + 2; if (s > 99) break }; s }
  c(g(), s)
}
print(br())
# a local of the same name: `<<-` assigns the enclosing one, reads see the local
lo <- function() {
  x <- 100
  g <- function() { x <- 1; for (i in 1:50) x <<- x + 1; x }
  c(g(), x)
}
print(lo())
lo2 <- function() {
  x <- 100
  g <- function() { for (i in 1:50) { x <<- x + 1 }; x <- 0; for (i in 1:50) x <<- x + 1; x }
  c(g(), x)
}
print(lo2())
# the binding two frames out, a global, one that does not exist yet
gp <- function() {
  z <- 0
  mid <- function() { inner <- function() { for (i in 1:300) z <<- z + 1 }; inner(); z }
  c(mid(), z)
}
print(gp())
glob <- 0
gl <- function() { for (i in 1:300) glob <<- glob + 0.5; glob }
print(gl()); print(glob)
nw <- function() { for (i in 1:300) fresh_one <<- i; fresh_one }
print(nw()); print(fresh_one); rm(fresh_one)
# locked and active bindings stay with the VM
lk <- 0; lockBinding("lk", globalenv())
lf <- function() { for (i in 1:300) lk <<- lk + 1 }
print(tryCatch(lf(), error = function(e) conditionMessage(e))); print(lk)
unlockBinding("lk", globalenv())
cnt2 <- 0
makeActiveBinding("ab", function(v) if (missing(v)) cnt2 else cnt2 <<- v * 10, globalenv())
af <- function() { for (i in 1:30) ab <<- 1 }
af(); print(cnt2); rm(ab)
# a value of another type
ty <- function() {
  x <- 0L
  g <- function() { for (i in 1:300) { x <<- x + 1L; if (i == 200) x <<- 0.5 }; x }
  c(g(), x)
}
print(ty())
# a promise (a formal) and a function as the binding
pf <- function(p) { g <- function() { for (i in 1:300) p <<- p + 1; p }; c(g(), p) }
print(pf(1))
fn <- function() { h <- 1; g <- function() for (i in 1:300) h <<- h * 1; g(); h }
print(fn())
