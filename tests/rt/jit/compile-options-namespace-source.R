# INTERIM(L5) (recorded, not ratcheted: M1's loader, the default boot, binds
# packages' code from source, and robigo-rt reads no compilerOptions for a
# closure under a namespace there; the rdb boot reads them as GNU R): a
# closure of source made at run time under a namespace is compiled by the
# JIT as any other, its compilerOptions read.
o <- compiler:::compilerOptions
rm("optimize", envir = o)
makeActiveBinding("optimize", function() { cat("read\n"); 2 }, o)
f <- function() { for (i in 1:2) NULL; 7 }
environment(f) <- new.env(parent = asNamespace("compiler"))
print(f())
print(f())
