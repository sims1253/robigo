# eval.c's Exec and Tailcall (do_tailcall, handle_exec_continuation).
f <- function(...) tryCatch(..., error = function(e) list(conditionMessage(e), class(e), conditionCall(e)))
count <- function(n, acc) { force(acc); if (n == 0) acc else Tailcall(count, n - 1, acc + 1) }
print(count(300, 0))
# mutual recursion through Exec and an explicit environment
ev <- function(n) if (n == 0) TRUE else Exec(as.call(list(as.name("od"), n - 1)), globalenv())
od <- function(n) if (n == 0) FALSE else Tailcall(ev, n - 1)
print(ev(101))
# the callee's frame replaces the caller's
depth <- function(n) if (n == 0) sys.nframe() else Tailcall(depth, n - 1)
print(depth(10) == depth(0))
outer <- function() inner1()
inner1 <- function() Tailcall(inner2)
inner2 <- function() sys.calls()
print(outer())
pf <- function() Tailcall(pg)
pg <- function() parent.frame()
print(identical(pf(), globalenv()))
# FUN as a string, and sys.function()
sq <- function(x) x^2
print((function() Tailcall("sq", 3))())
down <- function(n) if (n <= 0) "done" else Tailcall(sys.function(), n - 1)
print(down(50))
# named arguments and laziness
nm <- function(a, b) Tailcall(nm2, b = a, a = b)
nm2 <- function(a, b) c(a = a, b = b)
print(nm(1, 2))
lz <- function(x) Tailcall(lz2, stop("not forced"))
lz2 <- function(y) "lazy"
print(lz(1))
# not in tail position: evaluated where it stands
nt <- function(n) { r <- Tailcall(nf, n); c(r, sys.nframe()) }
nf <- function(n) sys.nframe()
print(nt(1))
nt2 <- function(n) 1 + Tailcall(nf, n)
print(nt2(1))
# the same call text twice in one body, once in tail position
tw <- function(n) { a <- Tailcall(nf, n); b <- Tailcall(nf, n) }
print(tw(1))
tw2 <- function(n) { a <- Tailcall(nf, n); Tailcall(nf, n) }
print(c(tw2(1), (function() { a <- Tailcall(nf, 1); a })()))
# if branches and braces
br <- function(n) { if (n > 0) { Tailcall(nf, n) } else Tailcall(nf, -n) }
print(c(br(1), br(-1)))
# with on.exit: no jump
oe <- function() { on.exit(cat("exit\n")); Tailcall(nf, 1) }
print(oe())
# continuations that are not closures
print((function() Tailcall(sum, 1, 2, 3))())
print((function() Tailcall(`+`, 1, 2))())
print(withVisible((function() Tailcall(invisible, 7))()))
print(withVisible((function() Tailcall(identity, 7))()))
print((function() Exec(quote(paste("a", "b"))))())
print((function() Exec(expression(1 + 2)))())
e <- new.env(); assign("v", 42, envir = e)
print(f((function() Exec(quote(v), e))()))
print((function() Exec(quote(get("v")), e))())
# S3 methods
gen <- function(x, n) UseMethod("gen")
gen.default <- function(x, n) if (n == 0) "base" else Tailcall(gen, structure(1, class = "k"), n - 1)
gen.k <- function(x, n) if (n == 0) "k" else Tailcall(gen, 1, n - 1)
print(c(gen(1, 0), gen(1, 1), gen(1, 50)))
# errors
print(f((function() Tailcall())()))
print(f((function() Exec())()))
print(f((function() Exec(1))()))
print(f((function() Exec("f()"))()))
print(f((function() Tailcall(nosuchfunction, 1))()))
print(f((function() Tailcall(1, 2))()))
print(f(Tailcall(nf, 1)))
print(f(Exec(quote(nf(1)))))
# a value computed after a tail call
af <- function(n) if (n == 0) list(sys.nframe(), sys.call()) else Tailcall(af, n - 1)
print(af(3))
# the same calls from compiled code (closures compile from their second call)
run <- function() list(count(30, 0), ev(11), depth(10) == depth(0), outer()[[2]], down(20), nm(1, 2),
                       nt(1), nt2(1), tw(1), tw2(1), br(-1), gen(1, 3), af(2),
                       withVisible((function() Tailcall(invisible, 7))())$visible)
for (k in 1:3) r <- run()
print(r)
# Exec's arguments are evaluated as evalListKeepMissing does: each value
# referenced until both are, `...` expanded
la <- function() "a"; lb <- function() "b"
lk <- function() { x <- as.call(list(as.name("la"))); Exec(x, { x[[1]] <- as.name("lb"); environment() }) }
print(lk())
fw <- function(...) Exec(...)
print(fw(quote(sum(1, 2))))
print(fw(quote(sum(1, 2)), globalenv()))
print(f(fw()))
# an alias calls the same primitive: the compiled pool shares its call too
tc <- Tailcall; gn <- function() sys.nframe()
al <- function() { tc(gn); tc(gn) }
for (k in 1:3) print(al())
