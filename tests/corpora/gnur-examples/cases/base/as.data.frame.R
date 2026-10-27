### Name: as.data.frame
### Title: Coerce to a Data Frame
### Aliases: as.data.frame is.data.frame as.data.frame.AsIs
###   as.data.frame.character as.data.frame.data.frame
###   as.data.frame.default as.data.frame.list as.data.frame.matrix
###   as.data.frame.model.matrix as.data.frame.ts as.data.frame.vector
###   as.data.frame.array
### Keywords: classes methods

### ** Examples

L0 <- list(LETTERS[1:7], c(4L, 2:3, 5:7, 1L))
L <- L0; names(L) <- nms <- c("nam", "age")
 d0 <- as.data.frame(L0, col.names = nms)
(d1 <- as.data.frame(L))
stopifnot(identical(d0, d1))

## showing possibilities  on how NA names are handled:
L <- list(A = 1:4); names(L) <- NA
names(dL  <- as.data.frame(L))                                          # "NA."
names(dL1 <- as.data.frame(L, col.names = names(L)))                    # "NA."
##
names(dL1.<- as.data.frame(L,                       check.names=FALSE)) # "NA"
##
names(dL2 <- as.data.frame(L, col.names = names(L), check.names=FALSE)) #  NA
names(dLn <- as.data.frame(L, new.names = TRUE,     check.names=FALSE)) #  NA



