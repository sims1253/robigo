# Closures that `function` makes from one byte-code body share it (GNU R's
# MKCLOSURE shares the BCODESXP), and with it robigo-rt's translation of it:
# each keeps its own environment, and a closure whose formals change gets
# code for its own.
mk <- compiler::cmpfun(function(a) function(x, y = a) x + y + a)
f1 <- mk(1); f10 <- mk(10)
print(c(f1(1), f10(1), f1(1, 2), f10(1, 2)))
print(identical(body(f1), body(f10)))
print(typeof(.Internal(bodyCode(f1))))
# many closures from one literal
fs <- lapply(1:5, function(i) mk(i))
print(vapply(fs, function(f) f(0), 0))
# one closure's formals rewritten: its own code, the others unchanged
g <- mk(2)
formals(g) <- alist(x = , y = 100, z = 0)
print(c(g(1), f1(1), f10(1)))
