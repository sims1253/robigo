RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: aregexec
### Title: Approximate String Match Positions
### Aliases: aregexec
### Keywords: character

### ** Examples

## Cf. the examples for agrep.
x <- c("1 lazy", "1", "1 LAZY")
aregexec("laysy", x, max.distance = 2)
aregexec("(lay)(sy)", x, max.distance = 2)
aregexec("(lay)(sy)", x, max.distance = 2, ignore.case = TRUE)
m <- aregexec("(lay)(sy)", x, max.distance = 2)
regmatches(x, m)



