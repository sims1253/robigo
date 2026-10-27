### Name: setLoadActions
### Title: Set Actions For Package Loading
### Aliases: setLoadAction setLoadActions getLoadActions hasLoadAction
###   evalOnLoad evalqOnLoad
### Keywords: package

### ** Examples

## Not run: 
##D ## in the code for some package
##D 
##D ## ... somewhere else
##D setLoadActions(function(ns)
##D    cat("Loaded package", sQuote(getNamespaceName(ns)),
##D        "at", format(Sys.time()), "\n"),
##D   setCount = function(ns) assign("myCount", 1, envir = ns),
##D   function(ns) assign("myPointer", getMyExternalPointer(), envir = ns))
##D   ... somewhere later
##D if(countShouldBe0)
##D   setLoadAction(function(ns) assign("myCount", 0, envir = ns), "setCount")
## End(Not run)



