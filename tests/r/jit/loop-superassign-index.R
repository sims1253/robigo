# A loop that assigns elements with `x[[i]] <<- v` runs natively on the
# vector of the binding `<<-` assigns (jit.rs, vm::jit_enter), and short
# loops entered often compile once their iterations add up (to
# vm::JIT_HEAT: the loops below run that often).
bx <- c(1, 2, 3, 4, 5); bv <- c(0.5, -0.5, 0.25, 1, 2); bm <- c(10, 20, 30, 40, 50)
step <- function(dt) {
  for (i in 1:5) bx[[i]] <<- bx[[i]] + dt * bv[[i]]
  for (i in 1:4) for (j in (i + 1):5) {
    d <- bx[[i]] - bx[[j]]
    bv[[i]] <<- bv[[i]] - d * bm[[j]] * dt
    bv[[j]] <<- bv[[j]] + d * bm[[i]] * dt
  }
}
for (k in 1:400) step(0.001)
print(bx); print(bv)
alias <- bx
for (k in 1:10) step(0.001)
print(identical(alias, bx)); print(alias); print(bx)
# an enclosing function's vector, read back in the same loop
outer_fn <- function() {
  acc <- numeric(6)
  add <- function(k) for (i in 1:6) acc[[i]] <<- acc[[i]] + i * k + acc[[max(1, i - 1)]]
  for (k in 1:300) add(k %% 7)
  acc
}
print(outer_fn())
# integers, `[`, NA values and indices, extension, type changes
iv <- 1:5
inc <- function(by) for (i in 1:5) iv[i] <<- iv[i] + by
for (k in 1:300) inc(1L)
print(iv)
inc(NA_integer_); print(iv)
iv <- 1:5; for (k in 1:300) inc(2L); inc(0.5); print(iv)
dv <- c(1, 2, 3)
grow <- function(n) for (i in 1:n) dv[[i]] <<- i * 2
for (k in 1:400) grow(3)
grow(5); print(dv)
na_at <- function(ix) for (k in 1:20) dv[[ix]] <<- k
print(tryCatch(na_at(NA), error = function(e) conditionMessage(e)))
print(dv)
ovf <- function() for (i in 1:20) iv[[1]] <<- iv[[1]] + .Machine$integer.max
iv <- 1:5; ovf(); print(iv)
# a local binding of the same name, a missing or locked target
shadow <- function() {
  bx <- c(0, 0, 0, 0, 0)
  for (i in 1:5) bx[[i]] <<- -i
  bx
}
print(shadow()); print(bx)
nowhere <- function() for (i in 1:20) zz_nowhere[[i]] <<- i
print(tryCatch(nowhere(), error = function(e) conditionMessage(e)))
lk <- function() {
  e <- new.env(); assign("q", c(1, 2, 3), envir = e); lockBinding("q", e)
  f <- function() for (i in 1:20) q[[1]] <<- i
  environment(f) <- new.env(parent = e)
  print(tryCatch(f(), error = function(err) conditionMessage(err)))
  print(get("q", envir = e))
}
lk()
ab <- function() {
  e <- new.env(); store <- c(1, 2, 3)
  makeActiveBinding("av", function(v) if (missing(v)) store else store <<- v, e)
  f <- function() for (i in 1:20) av[[2]] <<- i
  environment(f) <- new.env(parent = e)
  f(); print(store)
}
ab()
# a short loop whose types change between entries
tc <- function(x) { s <- 0; for (i in seq_along(x)) s <- s + x[[i]]; s }
for (k in 1:600) r <- tc(c(1.5, 2.5))
print(r); print(tc(1:3)); print(tc(c(TRUE, NA))); print(tc(list(1, 2)))
