### Name: ns-load
### Title: Loading and Unloading Name Spaces
### Aliases: attachNamespace loadNamespace requireNamespace
###   loadedNamespaces unloadNamespace isNamespaceLoaded
### Keywords: data utilities

### ** Examples

 (lns <- loadedNamespaces())
 statL <- isNamespaceLoaded("stats")
 stopifnot( identical(statL, "stats" %in% lns) )

 ## The string "foo" and the symbol 'foo' can be used interchangeably here:
 stopifnot( identical(isNamespaceLoaded(  "foo"   ), FALSE),
            identical(isNamespaceLoaded(quote(foo)), FALSE),
            identical(isNamespaceLoaded(quote(stats)), statL))

hasS <- isNamespaceLoaded("splines") # (to restore if needed)
Sns <- asNamespace("splines") # loads it if not already
stopifnot(   isNamespaceLoaded("splines"))
if (is.null(try(unloadNamespace(Sns)))) # try unloading the NS 'object'
stopifnot( ! isNamespaceLoaded("splines"))
if (hasS) loadNamespace("splines") # (restoring previous state)



