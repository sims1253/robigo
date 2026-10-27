### Name: stop
### Title: Stop Function Execution
### Aliases: stop geterrmessage
### Keywords: environment programming error

### ** Examples

iter <- 12
try(if(iter > 10) stop("too many iterations"))

tst1 <- function(...) stop("dummy error")
try(tst1(1:10, long, calling, expression))

tst2 <- function(...) stop("dummy error", call. = FALSE)
try(tst2(1:10, longcalling, expression, but.not.seen.in.Error))



