### Name: grepRaw
### Title: Pattern Matching for Raw Vectors
### Aliases: grepRaw
### Keywords: utilities

### ** Examples

grepRaw("no match", "textText")  # integer(0): no match
grepRaw("adf", "adadfadfdfadadf") # 3 - the first match
grepRaw("adf", "adadfadfdfadadf", all=TRUE, fixed=TRUE)
## [1]  3  6 13 -- three matches



