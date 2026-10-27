### Name: getFromNamespace
### Title: Utility Functions for Developing Namespaces
### Aliases: assignInNamespace assignInMyNamespace getFromNamespace
###   fixInNamespace
### Keywords: data

### ** Examples

getFromNamespace("findGeneric", "utils")
## Not run: 
##D fixInNamespace("predict.ppr", "stats")
##D stats:::predict.ppr
##D getS3method("predict", "ppr")
##D ## alternatively
##D fixInNamespace("predict.ppr", pos = 3)
##D fixInNamespace("predict.ppr", pos = "package:stats")
## End(Not run)


