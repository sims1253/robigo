RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: numeric
### Title: Numeric Vectors
### Aliases: numeric as.numeric is.numeric is.numeric.Date
###   is.numeric.POSIXt
### Keywords: classes attribute

### ** Examples

## Conversion does trim whitespace; non-numeric strings give NA + warning
as.numeric(c("-.1"," 2.7 ","B"))

## Numeric values are sometimes accidentally converted to factors.
## Converting them back to numeric is trickier than you'd expect.
f <- factor(5:10)
as.numeric(f) # not what you might expect, probably not what you want
## what you typically meant and want:
as.numeric(as.character(f))
## the same, considerably more efficient (for long vectors):
as.numeric(levels(f))[f]



