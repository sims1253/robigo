library(grid)
### Name: grid.grab
### Title: Grab the current grid output
### Aliases: grid.grab grid.grabExpr
### Keywords: dplot

### ** Examples

pushViewport(viewport(width=.5, height=.5))
grid.rect()
grid.points(stats::runif(10), stats::runif(10))
popViewport()
grab <- grid.grab()
grid.newpage()
grid.draw(grab)



