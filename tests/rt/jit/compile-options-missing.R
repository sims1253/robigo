# A compilerOptions binding that is gone is get()'s error in the compile's
# reads (getCompilerOption's get(name, compilerOptions)): the compile
# fails, tryCmpfun's handler reports it (suppressAll FALSE), and the
# closure runs uncompiled.
o <- compiler:::compilerOptions
rm("suppressUndefined", envir = o)
assign("suppressAll", FALSE, envir = o)
f <- function() { for (i in 1:2) NULL; 42 }
print(f())
