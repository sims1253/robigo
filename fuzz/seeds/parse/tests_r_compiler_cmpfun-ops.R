# Ops group dispatch between two classes takes the method only when both
# methods are identical; eval.c:DispatchGroup compares them with flags
# 16 + 1 + 2 + 4, so byte code is ignored ("can change at runtime") and a
# compiled method matches its uncompiled source. Methods that differ
# warn and fall back to the internal operator.
Ops.a <- function(e1, e2) paste("Ops.a", .Generic)
Ops.b <- compiler::cmpfun(Ops.a)
x <- structure(1, class = "a")
y <- structure(2, class = "b")
x + y
y == x
Ops.d <- eval(parse(text = deparse(Ops.a)))
d <- structure(5, class = "d")
d * x
x - d
Ops.c <- function(e1, e2) paste("Ops.c", .Generic)
z <- structure(3, class = "c")
x + z
Ops.e <- compiler::cmpfun(Ops.c)
e <- structure(4, class = "e")
e > z
identical(Ops.a, Ops.b)
identical(Ops.a, Ops.b, ignore.bytecode = FALSE)
