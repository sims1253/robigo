### Name: chkDots
### Title: Warn About Extraneous Arguments in the "..." of Its Caller
### Aliases: chkDots
### Keywords: utilities

### ** Examples

seq.default ## <- you will see  ' chkDots(...) '

seq(1,5, foo = "bar") # gives warning via chkDots()

## warning with more than one ...-entry:
density.f <- function(x, ...) NextMethod("density")
x <- density(structure(rnorm(10), class="f"), bar=TRUE, baz=TRUE)



