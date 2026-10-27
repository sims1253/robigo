### Name: strcapture
### Title: Capture String Tokens into a data.frame
### Aliases: strcapture
### Keywords: utilities

### ** Examples

x <- "chr1:1-1000"
pattern <- "(.*?):([[:digit:]]+)-([[:digit:]]+)"
proto <- data.frame(chr=character(), start=integer(), end=integer())
strcapture(pattern, x, proto)



