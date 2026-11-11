# Tailcall()/Exec() in tail position replace the closure's frame
# (eval.c:do_tailcall, handle_exec_continuation): constant stack, and the
# frame stack seen by the callee no longer holds the caller.

# A long tail-recursive loop, far past options(expressions=).
count <- function(n, acc) { force(acc); if (n == 0) acc else Tailcall(count, n - 1, acc + 1) }
print(count(200000, 0))

# Mutual recursion, through Exec with an explicit environment.
ev <- function(n) if (n == 0) TRUE else Exec(call("od", n - 1), globalenv())
od <- function(n) if (n == 0) FALSE else Tailcall(ev, n - 1)
print(ev(100001))

# The callee's frame replaces the caller's: same depth, same parent.
depth <- function(n) if (n == 0) sys.nframe() else Tailcall(depth, n - 1)
print(depth(10) == depth(0))
outer <- function() inner1()
inner1 <- function() Tailcall(inner2)
inner2 <- function() sys.calls()
print(outer())
pf <- function() Tailcall(pf2)
pf2 <- function() parent.frame()
print(identical(pf(), globalenv()))

# The call is what Tailcall built; FUN may be a string.
sc <- function(x) Tailcall("sc2", x + 1, y = 2)
sc2 <- function(x, y) sys.call()
print(sc(1))

# The function is found before the jump: sys.function() names the caller.
sf <- function(n) if (n > 0) Tailcall(sys.function(), n - 1) else "done"
print(sf(5))

# Promised arguments are evaluated in the caller's frame, lazily.
lz <- function(a) { b <- 10; Tailcall(lz2, b + a, stop("never")) }
lz2 <- function(x, unused) x
print(lz(1))

# A builtin or special as FUN.
bi <- function(x) Tailcall(sum, x, 1)
print(bi(1:3))
sp <- function(x) Tailcall(quote, x + 1)
print(sp(2))

# Exec of an expression vector, in a given environment.
ex <- function() { e <- new.env(); assign("v", 42, e); Exec(expression(v + 1), e) }
print(ex())

# S3 methods jump too.
area <- function(s) UseMethod("area")
area.sq <- function(s) Tailcall(sqarea, s$side)
sqarea <- function(side) c(side^2, sys.nframe())
print(area(structure(list(side = 3), class = "sq")))

# on.exit code blocks the jump: the call nests, and the code still runs.
oe <- function() { on.exit(cat("exit oe\n")); Tailcall(oe2) }
oe2 <- function() length(sys.calls())
print(oe())

# Visibility is the callee's.
inv <- function() Tailcall(invisible, 5)
inv()
print(withVisible(inv())$visible)

# Errors in the callee report the callee's call.
er <- function(x) Tailcall(er2, x)
er2 <- function(x) stop("bad ", x)
r <- tryCatch(er("y"), error = function(e) e)
print(conditionMessage(r))
print(conditionCall(r))

# An unknown function is reported at the jump.
uf <- function() Tailcall(no_such_fun, 1)
r <- tryCatch(uf(), error = function(e) e)
print(conditionMessage(r))

# A builtin continuation whose S3 method jumps again.
length.lz <- function(x) Tailcall(nchar, "abcd")
tl <- function(x) Tailcall(length, x)
print(tl(structure(1, class = "lz")))

# A continuation's body making a non-tail call that jumps itself.
nest <- function(n) { k <- count(1000, 0); Tailcall(nest2, n + k) }
nest2 <- function(m) c(m, sys.nframe())
print(nest(1))

# Out of tail position the call is evaluated as built: a string FUN is
# no function there.
st <- function() { y <- Tailcall("sc2", 1); y }
r <- tryCatch(st(), error = function(e) e)
print(conditionMessage(r))
