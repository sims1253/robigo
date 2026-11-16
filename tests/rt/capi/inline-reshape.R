# Formals written in place after a closure ran inline (docs/g3.md §9.3,
# §12): the next calls match the new formals, as GNU R's do. Each closure
# first runs in a compiled loop, where its calls run inline; then package
# C's SETCDR gives its formals a new tail.
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
