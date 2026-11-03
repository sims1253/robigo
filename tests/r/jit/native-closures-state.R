# Whole closures in native code (jit/closure.rs): variables of enclosing
# environments, `<<-`, vectors, cold paths, visibility.
hot <- function(f, ...) {
  a <- list(...)
  n <- length(a)
  if (n == 0) for (i in 1:600) f()
  else if (n == 1) { x <- a[[1]]; for (i in 1:600) f(x) }
  else if (n == 2) { x <- a[[1]]; y <- a[[2]]; for (i in 1:600) f(x, y) }
  else stop("hot")
}

# visibility of the last value
as1 <- function(n) x <- n
hot(as1, 1)
as1(1)
print(as1(2))
print(withVisible(as1(1))$visible)
as2 <- function(n) { y <- n + 1; y }
hot(as2, 1)
as2(1)
wv <- function(n) if (n > 0) { z <- n } else n
hot(wv, 1)
wv(1)
wv(-1)
cnt <- 0
inc <- function() cnt <<- cnt + 1
hot(inc)
inc()
print(cnt)

# `<<-` counters, in the global environment and in a frame
counter <- 0L
bump <- function(by) { counter <<- counter + by; counter }
hot(bump, 1L)
print(counter); print(bump(2L)); print(counter)
counter <- 2.5
print(bump(1)); print(counter); print(typeof(counter))
counter <- 2147483647L
r <- suppressWarnings(bump(1L)); print(r); print(counter)
mk <- function() { n <- 0; function() { n <<- n + 1; n } }
c1 <- mk(); c2 <- mk()
hot(c1); hot(c1); hot(c2)
print(c1()); print(c2())
# a run that is abandoned after writing leaves no trace: the VM's replay
# counts once
st <- 0
half <- function(a) { st <<- st + 1; if (a) 1 else 2 }
hot(half, TRUE)
print(st)
r <- tryCatch(half(NA), error = function(e) conditionMessage(e)); print(r)
print(st)
# ...also from deep in a tree
log1 <- 0
leaf <- function(x) { log1 <<- log1 + 1; if (x > 2) sqrt(-x) else x }
mid <- function(x) leaf(x) + leaf(x + 1)
hot(mid, 0)
print(log1)
r <- withCallingHandlers(mid(2), warning = function(w) { print(conditionMessage(w)); invokeRestart("muffleWarning") })
print(r); print(log1)
# two closures, one variable
sh <- 1
rd <- function() sh
wr <- function(v) { sh <<- v; rd() + 1 }
hot(wr, 1)
print(wr(5)); print(sh)
# `<<-` to a name without a binding creates a global
nw <- function() { fresh <<- 1; 2 }
hot(nw)
print(fresh); rm(fresh); nw(); print(fresh); rm(fresh)
# `<<-` to a formal of the enclosing function, a locked or active binding
outer1 <- function(p) { f <- function() { p <<- p + 1; p }; for (i in 1:600) f(); p }
print(outer1(1))
lk <- 1; lockBinding("lk", globalenv())
wl <- function() lk <<- 2
r <- tryCatch(hot(wl), error = function(e) conditionMessage(e)); print(r)
unlockBinding("lk", globalenv())
makeActiveBinding("ab", function(v) { if (!missing(v)) cat("set", v, "\n"); 1 }, globalenv())
wa <- function() { ab <<- 2; 3 }
for (i in 1:300) wa2 <- 1
hotq <- function(f) for (i in 1:2) f()
hotq(wa)
rm(ab)
# local shadowing `<<-`
ls1 <- 10
sl <- function() { ls1 <- 1; ls1 <<- ls1 + 1; ls1 }
hot(sl)
print(sl()); print(ls1)
# a global read inside a loop that a callee changes
tick <- 0
step <- function() tick <<- tick + 1
loop <- function(n) { for (i in 1:n) { step() }; tick }
hot(loop, 2)
print(loop(3)); print(tick)
vmcache <- function() { for (i in 1:600) step(); tick }
print(vmcache())

# closures sharing bytecode, different environments
adder <- function(k) function(x) x + k
a1 <- adder(1); a2 <- adder(100)
hot(a1, 1); hot(a2, 1)
print(a1(1)); print(a2(1))
a3 <- adder("no")
r <- tryCatch(a3(1), error = function(e) conditionMessage(e)); print(r)
a4 <- adder(1L)
print(a4(1L)); print(typeof(a4(1L))); print(a4(1.5))
# a helper redefined by each call of its enclosing function
out <- function(n) { h <- function(x) x * n; s <- 0; for (i in 1:50) s <- s + h(i); s }
hot(out, 2)
print(out(3)); print(out(2L))
# a frame variable that changes between calls
vf <- function() { b <- 1; g <- function() b + 1; r <- 0; for (i in 1:300) { b <- i; r <- r + g() }; r }
print(vf())
# a frame binding appears and shadows the global function
gl <- function(x) x + 1
us <- function(x) gl(x) * 2
hot(us, 1)
sc <- function() { gl <- function(x) x + 1000; us2 <- us; environment(us2) <- environment(); us2(1) }
print(sc()); print(us(1))

# a cold path with an op native code does not have
ck <- function(x) { if (x < 0) stop("negative: ", x); sqrt(x) }
hot(ck, 4)
print(ck(9))
r <- tryCatch(ck(-1), error = function(e) { print(conditionCall(e)); conditionMessage(e) }); print(r)
ct <- function(x) { if (x > 100) cat("big\n"); x + 1 }
hot(ct, 1)
print(ct(1)); print(ct(101))
rr <- function(x) { if (x > 0) x else paste("no", x) }
hot(rr, 1)
print(rr(1)); print(rr(-1))
# ...that becomes the hot path
flip <- function(x) { if (x > 0) x else nchar("abc") + x }
hot(flip, 1)
for (i in 1:1000) r <- flip(-1)
print(r); print(flip(1))

# vectors
v <- c(1.5, 2.5, 3.5); w <- 1:3
gv <- function(i) v[i]
hot(gv, 2)
print(gv(1)); print(gv(3L)); print(gv(4)); print(gv(0)); print(gv(-1)); print(gv(NA)); print(gv(2.9))
v <- 1:3
print(gv(2)); print(typeof(gv(2)))
v <- list(1, 2, 3)
print(gv(2))
v <- c(a = 1, b = 2, c = 3)
print(gv(2))
v <- c(1.5, 2.5, 3.5)
sm <- function(x) { s <- 0; for (i in 1:length(x)) s <- s + x[[i]]; s }
hot(sm, v)
print(sm(v)); print(sm(1:4)); print(sm(c(TRUE, FALSE)))
r <- tryCatch(sm(numeric(0)), error = function(e) conditionMessage(e)); print(r)
r <- tryCatch(sm(list(1, "a")), error = function(e) conditionMessage(e)); print(r)
print(sm(c(1, NA)))
lg <- function(x) length(x) + 1L
hot(lg, v)
print(lg(v)); print(lg(1)); print(lg(1:10)); print(lg(NULL)); print(lg(list(1)))
cl <- structure(c(1, 2), class = "foo"); length.foo <- function(x) 99L
print(lg(cl))
nl <- function(x) is.null(x)
hot(nl, v)
print(nl(v)); print(nl(NULL)); print(nl(1))
m <- matrix(1:6 + 0.5, 2)
me <- function(i, j) m[i, j]
hot(me, 1, 2)
print(me(2, 3)); print(me(1L, 1L))
r <- tryCatch(me(3, 1), error = function(e) conditionMessage(e)); print(r)
pa <- function(x, i) x[i] + x[[i]]
hot(pa, v, 2)
print(pa(v, 1)); print(pa(1:3, 2L))
r <- tryCatch(pa(v, 5), error = function(e) conditionMessage(e)); print(r)

# loops in closures
ln <- function(n) { s <- 0L; for (i in seq_len(n)) s <- s + i; s }
hot(ln, 3)
print(ln(4)); print(ln(0)); print(ln(4L))

# an argument is read when the callee forces it, after its `<<-`
na <- 0
fa <- function(x) { na <<- na + 1; x }
ga <- function() fa(na)
for (i in 1:600) r <- ga()
print(r); print(na)
for (i in 1:600) r <- fa(na)
print(r); print(na)
nb <- 0
fb <- function(x) { nb <<- nb + 1; x + 1 }
for (i in 1:600) r <- fb(nb + 1)
print(r); print(nb)
# assignments still pending in the caller's frame, at the first native entry
pe <- function() {
  b <- 1
  g <- function() { z <- b + 1; z }
  s <- 0
  for (i in 1:600) {
    b <- i
    r <- g()
    s <- s + r
  }
  s
}
print(pe())
# `<<-` of a value of another type
ty <- 1L
ft <- function(flag) { if (flag) ty <<- 2.5; ty }
for (i in 1:601) {
  v <- i > 600
  r <- ft(v)
}
print(r); print(ty); print(typeof(ty))
tb <- 1
fl <- function(flag) { if (flag) tb <<- TRUE; tb }
for (i in 1:601) { v <- i > 600; r <- fl(v) }
print(r); print(tb); print(typeof(tb))
# a variable's value is visible after an invisible nested call
hi <- function(x) y <- x
gi <- function(x) { hi(x); x }
run <- function() {
  for (i in 1:600) gi(1)
  gi(1)
}
print(withVisible(run())$visible)
run()
ki <- function(x) { hi(x); 5 }
ri <- function() { for (i in 1:600) ki(1); ki(1) }
print(withVisible(ri())$visible)
ai <- function(x) { hi(x); x + 1 }
qi <- function() { for (i in 1:600) ai(1); ai(1) }
print(withVisible(qi())$visible)
ti <- function(x) { hi(x); x > 0 }
wi <- function() { for (i in 1:600) ti(1); ti(1) }
print(withVisible(wi())$visible)
ii <- function(x, v) { hi(x); v[1] }
si <- function() { v <- c(1, 2); for (i in 1:600) ii(1, v); ii(1, v) }
print(withVisible(si())$visible)
