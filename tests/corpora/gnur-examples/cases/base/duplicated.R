RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: duplicated
### Title: Determine Duplicate Elements
### Aliases: duplicated duplicated.default duplicated.data.frame
###   duplicated.matrix duplicated.array anyDuplicated
###   anyDuplicated.default anyDuplicated.array anyDuplicated.matrix
###   anyDuplicated.data.frame
### Keywords: logic manip

### ** Examples

x <- c(9:20, 1:5, 3:7, 0:8)
## extract unique elements
(xu <- x[!duplicated(x)])
## similar, same elements but different order:
(xu2 <- x[!duplicated(x, fromLast = TRUE)])

## xu == unique(x) but unique(x) is more efficient
stopifnot(identical(xu,  unique(x)),
          identical(xu2, unique(x, fromLast = TRUE)))

which(duplicated(warpbreaks))

## for 3d array, MARGIN = -1  <==>  MARGIN = 2:3
duplicated(gait, MARGIN = -1) # boy26 duplicates boy19, see help(gait).
anyDuplicated(warpbreaks) 
## Don't show: 
stopifnot(identical(anyDuplicated(warpbreaks), 8L),
          identical(anyDuplicated(gait, MARGIN = -1), 26L))
## End(Don't show)
anyDuplicated(x)
anyDuplicated(x, fromLast = TRUE)



