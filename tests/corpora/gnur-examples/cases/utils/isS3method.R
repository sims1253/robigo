### Name: isS3method
### Title: Is 'method' the Name of an S3 Method?
### Aliases: isS3method
### Keywords: methods

### ** Examples

isS3method("t")           # FALSE - it is an S3 generic
isS3method("t.default")   # TRUE
isS3method("t.ts")        # TRUE
isS3method("t.test")      # FALSE
isS3method("t.data.frame")# TRUE
isS3method("t.lm")        # FALSE - not existing
isS3method("t.foo.bar")   # FALSE - not existing

## S3 methods with "4 parts" in their name:
ff <- c("as.list", "as.matrix", "is.na", "row.names", "row.names<-")
for(m in ff) if(isS3method(m)) stop("wrongly declared an S3 method: ", m)
(m4 <- paste(ff, "data.frame", sep="."))
for(m in m4) if(!isS3method(m)) stop("not an S3 method: ", m)
## Don't show: 
stopifnot(
  !isS3method("t"), !isS3method("t.test"), !isS3method("qr.coef"), !isS3method("sort.list"),
  isS3method("t.default"), isS3method("t.ts"), isS3method("t.data.frame"),
  !isS3method("t.lm"), !isS3method("t.foo.bar"))
## End(Don't show)



