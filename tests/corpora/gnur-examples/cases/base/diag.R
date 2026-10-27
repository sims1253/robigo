### Name: diag
### Title: Matrix Diagonals
### Aliases: diag diag<-
### Keywords: array

### ** Examples

dim(diag(3))
diag(10, 3, 4) # guess what?
all(diag(1:3) == {m <- matrix(0,3,3); diag(m) <- 1:3; m})

## other "numeric"-like diagonal matrices :
diag(c(1i,2i))    # complex
diag(TRUE, 3)     # logical
diag(as.raw(1:3)) # raw
(D2 <- diag(2:1, 4)); typeof(D2) # "integer"

require(stats)
## diag(<var-cov-matrix>) = variances
diag(var(M <- cbind(X = 1:5, Y = rnorm(5))))
#-> vector with names "X" and "Y"
rownames(M) <- c(colnames(M), rep("", 3))
M; diag(M) #  named as well
diag(M, names = FALSE) # w/o names
## Don't show: 
stopifnot(identical(names(diag(M)), colnames(M)),
                    is.null(names(diag(M,      names=FALSE))),
                    is.null(names(diag(var(M), names=FALSE)))) 
## End(Don't show)



