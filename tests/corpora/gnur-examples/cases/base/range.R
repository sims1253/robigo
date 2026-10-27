RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: range
### Title: Range of Values
### Aliases: range range.default range.Date range.POSIXct .rangeNum
### Keywords: univar arith

### ** Examples

(r.x <- range(stats::rnorm(100)))
diff(r.x) # the SAMPLE range

x <- c(NA, 1:3, -1:1/0); x
range(x)
range(x, na.rm = TRUE)
range(x, finite = TRUE)



