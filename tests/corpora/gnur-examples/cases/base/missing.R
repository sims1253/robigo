RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: missing
### Title: Does a Formal Argument have a Value?
### Aliases: missing
### Keywords: programming

### ** Examples

myplot <- function(x, y) {
                if(missing(y)) {
                        y <- x
                        x <- 1:length(y)
                }
                plot(x, y)
        }



