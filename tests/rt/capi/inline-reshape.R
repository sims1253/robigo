# Formals written in place after a closure ran inline (docs/g3.md §9.3,
# §12): the next calls match the new formals, as GNU R's do. Each closure
# first runs in a compiled loop, where its calls run inline; then package
# C's SETCDR gives its formals a new tail. Last, the references a
# reassigned formal's promise keeps (robigo does not inline such bodies).
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/pairlist.so"))
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/closure_mutation.so"))
grow <- function(f, tail) invisible(.Call("capi_setcdr_alias", .Call("rt_formals", f), as.pairlist(tail)))
# more formals than an inline call binds
g <- function(x) x + 1
run1 <- function() { s <- 0; for (i in 1:3) s <- s + g(i); s }
for (k in 1:3) r <- run1()
print(r)
grow(g, list(a = 1, b = 2, c = 3, d = 4))
for (k in 1:3) r <- run1()
print(r)
print(names(.Call("rt_formals", g)))
# a new formal named like the function the body calls
sq <- function(x) x * x
f <- function(x) sq(x)
run2 <- function() { s <- 0; for (i in 1:3) s <- s + f(i); s }
for (k in 1:3) r <- run2()
print(r)
grow(f, list(sq = function(x) -x))
for (k in 1:3) r <- run2()
print(r)
# a new formal named like the global the body reads
y <- 100
h <- function(x) x + y
run3 <- function() { s <- 0; for (i in 1:3) s <- s + h(i); s }
for (k in 1:3) r <- run3()
print(r)
grow(h, list(y = 7))
for (k in 1:3) r <- run3()
print(r)
# and passed by name
run4 <- function() h(1, y = 2)
for (k in 1:3) r <- run4()
print(r)
# a new formal named like a local the body assigns on one branch only
y <- 100
lo <- function(x) { if (x) y <- 1; y }
run5 <- function() c(lo(FALSE), lo(TRUE))
for (k in 1:3) r <- run5()
print(r)
grow(lo, list(y = 7))
for (k in 1:3) r <- run5()
print(r)
# a formal's constant default, never forced, overwritten by the body: the
# real call's promise for it keeps the default once the store drops it, so
# the default is shared after the formals let it go (TRUE: the write
# copies; C holds the old object while it compares, #709)
dyn.load(paste0(Sys.getenv("RT_CAPI_LIBS"), "/identity.so"))
ov <- function(a = NULL) { a <- 1; 0 }
for (k in 1:4) invisible(ov())
x <- c(1, 2, 3)
invisible(.Call("rt_set_formal_default", ov, 0L, x))
invisible(ov(3))
for (k in 1:4) invisible(ov())
invisible(.Call("rt_set_formal_default", ov, 0L, NULL))
print(.Call("rt_assign_copies", quote(x), quote(x[1] <- 5), environment()))
# and forced first
ovf <- function(a = NULL) { b <- a; a <- 1; 0 }
for (k in 1:4) invisible(ovf())
x <- c(1, 2, 3)
invisible(.Call("rt_set_formal_default", ovf, 0L, x))
invisible(ovf(3))
for (k in 1:4) invisible(ovf())
invisible(.Call("rt_set_formal_default", ovf, 0L, NULL))
print(.Call("rt_assign_copies", quote(x), quote(x[1] <- 5), environment()))
# a forced argument overwritten: the promise keeps its value
og <- function(v) { w <- v; v <- 1; w }
x <- c(1, 2, 3)
run6 <- function() og(x)
for (k in 1:3) r <- run6()
print(.Call("rt_assign_copies", quote(x), quote(x[1] <- 5), environment()))
