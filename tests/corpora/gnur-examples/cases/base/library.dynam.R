### Name: library.dynam
### Title: Loading DLLs from Packages
### Aliases: library.dynam library.dynam.unload .dynLibs
### Keywords: data

### ** Examples

## Which DLLs were dynamically loaded by packages?
library.dynam()

## More on library.dynam.unload() :
## No test: 
##D require(nlme)
##D nlme:::.onUnload # shows library.dynam.unload() call
##D detach("package:nlme")  # by default, unload=FALSE ,  so,
##D tail(library.dynam(), 2)# nlme still there
##D 
##D ## How to unload the DLL ?
##D ## Best is to unload the namespace,  unloadNamespace("nlme")
##D ## If we need to do it separately which should be exceptional:
##D pd.file <- attr(packageDescription("nlme"), "file")
##D library.dynam.unload("nlme", libpath = sub("/Meta.*", '', pd.file))
##D tail(library.dynam(), 2)# 'nlme' is gone now
##D unloadNamespace("nlme") # now gives warning
## End(No test)



