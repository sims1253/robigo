RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: SHLIB
### Title: Build Shared Object/DLL for Dynamic Loading
### Aliases: SHLIB
### Keywords: utilities

### ** Examples
## Not run: 
##D # To link against a library not on the system library paths:
##D R CMD SHLIB -o mylib.so a.f b.f -L/opt/acml3.5.0/gnu64/lib -lacml
## End(Not run)


