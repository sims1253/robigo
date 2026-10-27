### Name: classesToAM
### Title: Compute an Adjacency Matrix for Superclasses of Class
###   Definitions
### Aliases: classesToAM
### Keywords: classes programming

### ** Examples


## the super- and subclasses of "standardGeneric"
## and "derivedDefaultMethod"
am <- classesToAM(list(class(show), class(getMethod(show))), TRUE)
am

## Not run: 
##D ## the following function depends on the Bioconductor package Rgraphviz
##D plotInheritance <- function(classes, subclasses = FALSE, ...) {
##D     if(!require("Rgraphviz", quietly=TRUE))
##D       stop("Only implemented if Rgraphviz is available")
##D     mm <- classesToAM(classes, subclasses)
##D     classes <- rownames(mm); rownames(mm) <- colnames(mm)
##D     graph <-  new("graphAM", mm, "directed", ...)
##D     plot(graph)
##D     cat("Key:\n", paste(abbreviate(classes), " = ", classes, ", ",
##D         sep = ""),  sep = "", fill = TRUE)
##D     invisible(graph)
##D }
##D 
##D ## The plot of the class inheritance of the package "graph"
##D require(graph)
##D plotInheritance(getClasses("package:graph"))
##D 
## End(Not run)



