RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: lines
### Title: Add Connected Line Segments to a Plot
### Aliases: lines lines.default
### Keywords: aplot

### ** Examples

# draw a smooth line through a scatter plot
plot(cars, main = "Stopping Distance versus Speed")
lines(stats::lowess(cars))



