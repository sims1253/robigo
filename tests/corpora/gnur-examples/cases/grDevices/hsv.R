RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: hsv
### Title: HSV Color Specification
### Aliases: hsv
### Keywords: color dplot

### ** Examples

require(graphics)

hsv(.5,.5,.5)

## Red tones:
n <- 20;  y <- -sin(3*pi*((1:n)-1/2)/n)
op <- par(mar = rep(1.5, 4))
plot(y, axes = FALSE, frame.plot = TRUE,
     xlab = "", ylab = "", pch = 21, cex = 30,
     bg = rainbow(n, start = .85, end = .1),
     main = "Red tones")
par(op)



