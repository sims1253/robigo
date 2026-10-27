RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: getAnywhere
### Title: Retrieve an R Object, Including from a Namespace
### Aliases: getAnywhere argsAnywhere [.getAnywhere print.getAnywhere
### Keywords: data

### ** Examples

getAnywhere("format.dist")
getAnywhere("simpleLoess") # not exported from stats
argsAnywhere(format.dist)



