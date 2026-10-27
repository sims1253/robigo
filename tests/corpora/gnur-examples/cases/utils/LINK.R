RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: LINK
### Title: Create Executable Programs on Unix-alikes
### Aliases: LINK
### Keywords: utilities

### ** Examples
## Not run: 
##D ## examples of front-ends linked against R.
##D ## First a C program
##D CC=`R CMD config CC`
##D R CMD LINK $CC -o foo foo.o `R CMD config --ldflags`
##D 
##D ## if Fortran code has been compiled into ForFoo.o
##D FLIBS=`R CMD config FLIBS`
##D R CMD LINK $CC -o foo foo.o ForFoo.o `R CMD config --ldflags` $FLIBS
##D 
##D ## And for a C++ front-end
##D CXX=`R CMD config CXX`
##D R CMD COMPILE foo.cc
##D R CMD LINK $CXX -o foo foo.o `R CMD config --ldflags`
## End(Not run)


