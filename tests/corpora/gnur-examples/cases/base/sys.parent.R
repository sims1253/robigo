### Name: sys.parent
### Title: Functions to Access the Function Call Stack
### Aliases: sys.parent sys.call sys.calls sys.frame sys.frames sys.nframe
###   sys.function sys.parents sys.on.exit sys.status parent.frame
### Keywords: programming data

### ** Examples
## No test: 
##D require(utils)
##D 
##D ## Note: the first two examples will give different results
##D ## if run by example().
##D ff <- function(x) gg(x)
##D gg <- function(y) sys.status()
##D str(ff(1))
##D 
##D gg <- function(y) {
##D     ggg <- function() {
##D         cat("current frame is", sys.nframe(), "\n")
##D         cat("parents are", sys.parents(), "\n")
##D         print(sys.function(0)) # ggg
##D         print(sys.function(2)) # gg
##D     }
##D     if(y > 0) gg(y-1) else ggg()
##D }
##D gg(3)
##D 
##D t1 <- function() {
##D   aa <- "here"
##D   t2 <- function() {
##D     ## in frame 2 here
##D     cat("current frame is", sys.nframe(), "\n")
##D     str(sys.calls()) ## list with two components t1() and t2()
##D     cat("parents are frame numbers", sys.parents(), "\n") ## 0 1
##D     print(ls(envir = sys.frame(-1))) ## [1] "aa" "t2"
##D     invisible()
##D   }
##D   t2()
##D }
##D t1()
##D 
##D test.sys.on.exit <- function() {
##D   on.exit(print(1))
##D   ex <- sys.on.exit()
##D   str(ex)
##D   cat("exiting...\n")
##D }
##D test.sys.on.exit()
##D ## gives 'language print(1)', prints 1 on exit
##D 
##D ## An example where the parent is not the next frame up the stack
##D ## since method dispatch uses a frame.
##D as.double.foo <- function(x)
##D {
##D     str(sys.calls())
##D     print(sys.frames())
##D     print(sys.parents())
##D     print(sys.frame(-1)); print(parent.frame())
##D     x
##D }
##D t2 <- function(x) as.double(x)
##D a <- structure(pi, class = "foo")
##D t2(a)
## End(No test)


