library(methods)
### Name: evalSource
### Title: Use Function Definitions from a Source File without Reinstalling
###   a Package
### Aliases: evalSource insertSource sourceEnvironment-class
### Keywords: programming methods

### ** Examples

## Not run: 
##D ## Suppose package P0 has a source file "all.R"
##D ## First, evaluate the source, and from it
##D ## insert the revised version of methods for summary()
##D   env <- insertSource("./P0/R/all.R", package = "P0",
##D      methods = "summary")
##D ## now test one of the methods, tracing  the version from the source
##D   trace("summary", signature = "myMat", browser, edit = env)
##D ## After testing, remove the browser() call but keep the source
##D   trace("summary", signature = "myMat", edit = env)
##D ## Now insert all the (other) revised functions and methods
##D ## without re-evaluating the source file.
##D ## The package name is included in the object env.
##D   insertSource(env)
## End(Not run)



