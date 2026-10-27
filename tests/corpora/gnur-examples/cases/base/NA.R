### Name: NA
### Title: 'Not Available' / Missing Values
### Aliases: NA NA_integer_ NA_real_ NA_complex_ NA_character_ is.na
###   is.na.data.frame is.na<- is.na<-.default anyNA anyNA.data.frame
###   anyMissing
### Keywords: NA logic manip

### ** Examples

is.na(c(1, NA))        #> FALSE  TRUE
is.na(paste(c(1, NA))) #> FALSE FALSE

(xx <- c(0:4))
is.na(xx) <- c(2, 4)
xx                     #> 0 NA  2 NA  4
anyNA(xx) # TRUE

# Some logical operations do not return NA
c(TRUE, FALSE) & NA
c(TRUE, FALSE) | NA

## No test: 
##D ## Measure speed difference in a favourable case:
##D ## the difference depends on the platform, on most ca 3x.
##D x <- 1:10000; x[5000] <- NaN  # coerces x to be double
##D if(require("microbenchmark")) { # does not work reliably on all platforms
##D   print(microbenchmark(any(is.na(x)), anyNA(x)))
##D } else {
##D   nSim <- 2^13
##D   print(rbind(is.na = system.time(replicate(nSim, any(is.na(x)))),
##D               anyNA = system.time(replicate(nSim, anyNA(x)))))
##D }
## End(No test)

## anyNA() can work recursively with list()s:
LL <- list(1:5, c(NA, 5:8), c("A","NA"), c("a", NA_character_))
L2 <- LL[c(1,3)]
sapply(LL, anyNA); c(anyNA(LL), anyNA(LL, TRUE))
sapply(L2, anyNA); c(anyNA(L2), anyNA(L2, TRUE))

## ... lists, and hence data frames, too:
dN <- dd <- USJudgeRatings; dN[3,6] <- NA
anyNA(dd) # FALSE
anyNA(dN) # TRUE



