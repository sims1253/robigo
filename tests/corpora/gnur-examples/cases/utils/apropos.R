RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: apropos
### Title: Find Objects by (Partial) Name
### Aliases: apropos find
### Keywords: data documentation environment

### ** Examples

require(stats)

## Not run: apropos("lm")
apropos("GLM")                      # several
apropos("GLM", ignore.case = FALSE) # not one
apropos("lq")

cor <- 1:pi
find("cor")                         #> ".GlobalEnv"   "package:stats"
find("cor", numeric = TRUE)                     # numbers with these names
find("cor", numeric = TRUE, mode = "function")  # only the second one
rm(cor)

## Not run: apropos(".", mode = "list")  # includes many datasets

# extraction/replacement methods (need a DOUBLE backslash '\\')
apropos("\\[")

## No test: 
##D # everything % not diff-able
##D length(apropos("."))
##D 
##D # those starting with 'pr'
##D apropos("^pr")
##D 
##D # the 1-letter things
##D apropos("^.$")
##D # the 1-2-letter things
##D apropos("^..?$")
##D # the 2-to-4 letter things
##D apropos("^.{2,4}$")
##D # frequencies of 8-and-more letter things
##D table(nchar(apropos("^.{8,}$")))
## End(No test)


