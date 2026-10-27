RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: toTitleCase
### Title: Convert Titles to Title Case
### Aliases: toTitleCase

### ** Examples

toTitleCase("bayesian network modeling and analysis")
toTitleCase("ensemble tool for predictions from species distribution models")
## Treatment after "-":
toTitleCase("small- and large-scale analysis") # lowercase "and"

toTitleCase("a small fox is jumping")    # "a Small Fox is .."     (the 'a' may change)% i.e. BUG
toTitleCase("is a small fox jumping?")   # "Is a Small Fox .."     (fine)
## After ":", start a new sentence
toTitleCase("a pangram: the quick brown fox jumps over the lazy dog")
toTitleCase("asking -- 'is a small fox jumping?'") # ".. -- Is a Small ..."     (fine)



