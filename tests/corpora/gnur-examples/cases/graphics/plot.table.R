RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: plot.table
### Title: Plot Methods for 'table' Objects
### Aliases: plot.table lines.table points.table
### Keywords: hplot category

### ** Examples

## 1-d tables
(Poiss.tab <- table(N = stats::rpois(200, lambda = 5)))
plot(Poiss.tab, main = "plot(table(rpois(200, lambda = 5)))")

plot(table(state.division))

## 4-D :
plot(Titanic, main ="plot(Titanic, main= *)")




