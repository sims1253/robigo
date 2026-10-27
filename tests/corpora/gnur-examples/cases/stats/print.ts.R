RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: print.ts
### Title: Printing and Formatting of Time-Series Objects
### Aliases: .preformat.ts print.ts
### Keywords: ts

### ** Examples

print(ts(1:10, frequency = 7, start = c(12, 2)), calendar = TRUE)

print(sunsp.1 <- window(sunspot.m2014, end=c(1756, 12)))
m <- .preformat.ts(sunsp.1) # a character matrix



