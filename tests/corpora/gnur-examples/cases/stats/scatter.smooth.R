### Name: scatter.smooth
### Title: Scatter Plot with Smooth Curve Fitted by 'loess'
### Aliases: scatter.smooth loess.smooth
### Keywords: smooth

### ** Examples

require(graphics)

with(cars, scatter.smooth(speed, dist))
## or with dotted thick smoothed line results :
with(cars, scatter.smooth(speed, dist, lpars =
                    list(col = "red", lwd = 3, lty = 3)))



