RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: plot.raster
### Title: Plotting Raster Images
### Aliases: plot.raster
### Keywords: hplot

### ** Examples

require(grDevices)
r <- as.raster(c(0.5, 1, 0.5))
plot(r)
# additional arguments to rasterImage()
plot(r, interpolate=FALSE)
# distort
plot(r, asp=NA)
# fill page
op <- par(mar=rep(0, 4))
plot(r, asp=NA)
par(op)
# normal annotations work
plot(r, asp=NA)
box()
title(main="This is my raster")
# add to existing plot
plot(1)
plot(r, add=TRUE)



