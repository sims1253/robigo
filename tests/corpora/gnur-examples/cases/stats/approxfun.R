### Name: approxfun
### Title: Interpolation Functions
### Aliases: approx approxfun
### Keywords: arith dplot

### ** Examples

require(graphics)

x <- 1:10
y <- rnorm(10)
par(mfrow = c(2,1))
plot(x, y, main = "approx(.) and approxfun(.)")
points(approx(x, y), col = 2, pch = "*")
points(approx(x, y, method = "constant"), col = 4, pch = "*")

f <- approxfun(x, y)
curve(f(x), 0, 11, col = "green2")
points(x, y)
is.function(fc <- approxfun(x, y, method = "const")) # TRUE
curve(fc(x), 0, 10, col = "darkblue", add = TRUE)
## different extrapolation on left and right side :
plot(approxfun(x, y, rule = 2:1), 0, 11,
     col = "tomato", add = TRUE, lty = 3, lwd = 2)

### Treatment of 'NA's -- are kept if  na.rm=FALSE :

xn <- 1:4
yn <- c(1,NA,3:4)
xout <- (1:9)/2
## Default behavior (na.rm = TRUE): NA's omitted; extrapolation gives NA
data.frame(approx(xn,yn, xout))
data.frame(approx(xn,yn, xout, rule = 2))# -> *constant* extrapolation
## New (2019-2020)  na.rm = FALSE: NA's are "kept"
data.frame(approx(xn,yn, xout, na.rm=FALSE, rule = 2))
data.frame(approx(xn,yn, xout, na.rm=FALSE, rule = 2, method="constant"))

## NA's in x[] are not allowed:
stopifnot(inherits( try( approx(yn,yn, na.rm=FALSE) ), "try-error"))

## Give a nice overview of all possibilities  rule * method * na.rm :
##             -----------------------------  ====   ======   =====
## extrapolations "N":= NA;   "C":= Constant :
rules <- list(N=1, C=2, NC=1:2, CN=2:1)
methods <- c("constant","linear")
ry <- sapply(rules, function(R) {
       sapply(methods, function(M)
        sapply(setNames(,c(TRUE,FALSE)), function(na.)
                 approx(xn, yn, xout=xout, method=M, rule=R, na.rm=na.)$y),
        simplify="array")
   }, simplify="array")
names(dimnames(ry)) <- c("x = ", "na.rm", "method", "rule")
dimnames(ry)[[1]] <- format(xout)
ftable(aperm(ry, 4:1)) # --> (4 * 2 * 2) x length(xout)  =  16 x 9 matrix
## Don't show: 
ryna <- ry
ryna[is.na(ryna)] <- NA ## approx may turn NAs into NaNs
stopifnot(exprs = {
 identical(unname(ryna),
   array(c(NA, 1, 1,   1, 1, 3, 3, 4, NA,      NA, 1,  1, NA, NA, 3, 3, 4, NA,
           NA, 1, 1.5, 2, 2.5, 3, 3.5, 4, NA,  NA, 1, NA, NA, NA, 3, 3.5, 4, NA,
            1, 1, 1,   1, 1, 3, 3, 4, 4,        1, 1,  1, NA, NA, 3, 3, 4, 4,
            1, 1, 1.5, 2, 2.5, 3, 3.5, 4, 4,    1, 1, NA, NA, NA, 3, 3.5, 4, 4,
           NA, 1, 1,   1, 1, 3, 3, 4, 4,       NA, 1,  1, NA, NA, 3, 3, 4, 4,
           NA, 1, 1.5, 2, 2.5, 3, 3.5, 4, 4,   NA, 1, NA, NA, NA, 3, 3.5, 4, 4,
            1, 1, 1,   1, 1, 3, 3, 4, NA,       1, 1,  1, NA, NA, 3, 3, 4, NA,
            1, 1, 1.5, 2, 2.5, 3, 3.5, 4, NA,   1, 1, NA, NA, NA, 3, 3.5, 4, NA),
         dim = c(9L, 2L, 2L, 4L)))
 identical(approxfun(xn,yn, method="constant", rule=2, na.rm=FALSE)(xout),
            as.vector(ry[,"FALSE", "constant","C"]))
 identical(approxfun(xn,yn, method="linear", rule=2:1, na.rm=FALSE)(xout),
            as.vector(ry[,"FALSE", "linear", "CN"]))
})
## End(Don't show)

## Show treatment of 'ties' :

x <- c(2,2:4,4,4,5,5,7,7,7)
y <- c(1:6, 5:4, 3:1)
(amy <- approx(x, y, xout = x)$y) # warning, can be avoided by specifying 'ties=':
op <- options(warn=2) # warnings would be error
stopifnot(identical(amy, approx(x, y, xout = x, ties=mean)$y))
(ay <- approx(x, y, xout = x, ties = "ordered")$y)
stopifnot(amy == c(1.5,1.5, 3, 5,5,5, 4.5,4.5, 2,2,2),
          ay  == c(2, 2,    3, 6,6,6, 4, 4,    1,1,1))
approx(x, y, xout = x, ties = min)$y
approx(x, y, xout = x, ties = max)$y
options(op) # revert 'warn'ing level



