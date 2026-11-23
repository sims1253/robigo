# INTERIM(L5) (recorded, not ratcheted: M1's loader, the default boot,
# binds packages' code from source, and robigo-rt makes no findFunDef
# check for a closure under a namespace there; the rdb boot makes it as
# GNU R): a closure of source made at run time under a namespace is
# compiled by the JIT as any other, and cmpCall's findFunDef check of a
# call it does not inline forces a lazy binding of the name. (A loop:
# R_CheckJIT compiles a small closure outside the global environment only
# with one.)
e <- new.env()
delayedAssign("myfun", { cat("forced\n"); function(x) x }, assign.env = e)
attach(e, name = "lazy", warn.conflicts = FALSE)
f <- function(x) { for (i in 1:2) NULL; if (x) myfun(x); 7 }
environment(f) <- new.env(parent = asNamespace("compiler"))
cat("call 1\n"); print(f(FALSE))
cat("call 2\n"); print(f(FALSE))
