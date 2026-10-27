RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(grid)
### Name: grid.function
### Title: Draw a curve representing a function
### Aliases: grid.function functionGrob grid.abline
### Keywords: dplot

### ** Examples

    # abline
    # NOTE: in ROOT viewport on screen, (0, 0) at top-left
    #       and "native" is pixels!
    grid.function(function(x) list(x=x, y=0 + 1*x))
    # a more "normal" viewport with default normalized "native" coords
    grid.newpage()
    pushViewport(viewport())
    grid.function(function(x) list(x=x, y=0 + 1*x))
    # slightly simpler
    grid.newpage()
    pushViewport(viewport())
    grid.abline()
    # sine curve
    grid.newpage()
    pushViewport(viewport(xscale=c(0, 2*pi), yscale=c(-1, 1)))
    grid.function(function(x) list(x=x, y=sin(x)))
    # constrained sine curve
    grid.newpage()
    pushViewport(viewport(xscale=c(0, 2*pi), yscale=c(-1, 1)))
    grid.function(function(x) list(x=x, y=sin(x)),
                  range=0:1)
    # inverse sine curve
    grid.newpage()
    pushViewport(viewport(xscale=c(-1, 1), yscale=c(0, 2*pi)))
    grid.function(function(y) list(x=sin(y), y=y),
                  range="y")
    # parametric function
    grid.newpage()
    pushViewport(viewport(xscale=c(-1, 1), yscale=c(-1, 1)))
    grid.function(function(t) list(x=cos(t), y=sin(t)),
                  range=c(0, 9*pi/5))
    # physical abline
    grid.newpage()
    grid.function(function(x) list(x=x, y=0 + 1*x),
                  units="in")



