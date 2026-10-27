RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: delimMatch
### Title: Delimited Pattern Matching
### Aliases: delimMatch
### Keywords: character

### ** Examples

x <- c("\\value{foo}", "function(bar)")
delimMatch(x)
delimMatch(x, c("(", ")"))



