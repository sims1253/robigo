# GNU R's JIT decisions (R_CheckJIT, eval.c) seen through a constant
# argument's binding (docs/g3.md §9.1, E5): once the code making the call
# is byte code the argument is a value (PUSHCONSTARG), while the AST runs
# it a delayed promise.
bt <- function(a) .Internal(getBindingType(quote(a), environment()))
# a small top-level function: MAYBEJIT at its first call, compiled at its
# second
small <- function() bt(1)
print(small()); print(small()); print(small())
# a top-level function with a loop scores LOOP_JIT_SCORE: compiled at its
# first call
big <- function() { for (i in 1) NULL; bt(1) }
print(big()); print(big())
# a small function made by another function: NOJIT
mk <- function() function() bt(1)
inner <- mk()
print(inner()); print(inner()); print(inner())
# mk's second call compiles it: the closure its code makes is compiled
inner2 <- mk()
print(inner2())
# a function with a loop made by code GNU R never compiles (it may call
# browser()): MAYBEJIT at its first call, compiled at its second
mkbig <- function() { if (FALSE) browser(); function() { for (i in 1) NULL; bt(1) } }
lp <- mkbig()
print(lp()); print(lp()); print(lp())
print(mkbig()()); print(mkbig()())
# a top-level loop is compiled as it starts (R_compileAndExecute); the top
# level itself is the AST
for (i in 1:2) print(bt(1))
print(bt(1))
while (TRUE) { print(bt(1)); break }
repeat { print(bt(1)); break }
# a body that may call browser() is never compiled
nb <- function() { if (FALSE) browser(); bt(1) }
print(nb()); print(nb()); print(nb())
# environment<- returns a compiled closure to its source, and the JIT
# decides again
e <- function() bt(1)
print(e()); print(e())
environment(e) <- globalenv()
print(e()); print(e())
# a copy shares the compiled body; a copy made before compilation does not
cp <- function() bt(1)
print(cp())
cp2 <- cp
print(cp()); print(cp2())
