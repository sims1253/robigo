### Name: xy.coords
### Title: Extracting Plotting Structures
### Aliases: xy.coords
### Keywords: dplot

### ** Examples

ff <- stats::fft(1:9)
xy.coords(ff)
xy.coords(ff, xlab = "fft") # labels "Re(fft)",  "Im(fft)"
## Don't show: 
stopifnot(identical(xy.coords(ff, xlab = "fft"),
                    xy.coords(ff, ylab = "fft")))
xy.labs <- function(...) xy.coords(...)[c("xlab","ylab")]
stopifnot(identical(xy.labs(ff, xlab = "fft", setLab = FALSE),
                    list(xlab = "fft", ylab = "fft")),
          identical(xy.labs(ff, ylab = "fft", setLab = FALSE),
                    list(xlab = NULL, ylab = "fft")),
          identical(xy.labs(ff, xlab = "Re(fft)", ylab = "Im(fft)", setLab = FALSE),
                    list(xlab = "Re(fft)", ylab = "Im(fft)")))
## End(Don't show)
with(cars, xy.coords(dist ~ speed, NULL)$xlab ) # = "speed"

xy.coords(1:3, 1:2, recycle = TRUE) # otherwise error "lengths differ"
xy.coords(-2:10, log = "y")
##> xlab: "Index"  \\  warning: 3 y values <= 0 omitted ..
op <- options(warn = 2)# ==> warnings would be errors, we suppress the one "we know":
suppressWarnings(xy.coords(-2:10, log = "y"), classes="log_le_0") -> xy
options(op) # revert
stopifnot(is.list(xy), identical (1:13 +0,  xy$x),
          identical(c(rep(NA, 3), 1:10 +0), xy$y))



