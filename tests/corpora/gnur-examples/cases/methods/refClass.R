RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: ReferenceClasses
### Title: Objects With Fields Treated by Reference (OOP-style)
### Aliases: ReferenceClasses setRefClass getRefClass initFieldArgs
###   initRefFields activeBindingFunction-class
###   defaultBindingFunction-class uninitializedField-class
###   refClassRepresentation-class refObjectGenerator-class
###   refGeneratorSlot-class refClass-class refObject-class
###   refMethodDef-class refMethodDefWithTrace-class SuperClassMethod-class
###   show,envRefClass-method show,refMethodDef-method
###   show,externalRefMethod-method show,refClassRepresentation-method
###   externalRefMethod externalRefMethod-class
### Keywords: programming classes

### ** Examples

## a simple editor for matrix objects.  Method  $edit() changes some
## range of values; method $undo() undoes the last edit.
mEdit <- setRefClass("mEdit",
      fields = list( data = "matrix",
        edits = "list"))

## The basic edit, undo methods
mEdit$methods(
     edit = function(i, j, value) {
       ## the following string documents the edit method
       'Replaces the range [i, j] of the
        object by value.
        '
         backup <-
             list(i, j, data[i,j])
         data[i,j] <<- value
         edits <<- c(edits, list(backup))
         invisible(value)
     },
     undo = function() {
       'Undoes the last edit() operation
        and update the edits field accordingly.
        '
         prev <- edits
         if(length(prev)) prev <- prev[[length(prev)]]
         else stop("No more edits to undo")
         edit(prev[[1]], prev[[2]], prev[[3]])
         ## trim the edits list
         length(edits) <<- length(edits) - 2
         invisible(prev)
     })

## A method to automatically print objects
mEdit$methods(
     show = function() {
       'Method for automatically printing matrix editors'
       cat("Reference matrix editor object of class",
          classLabel(class(.self)), "\n")
       cat("Data: \n")
       methods::show(data)
       cat("Undo list is of length", length(edits), "\n")
     }
     )

xMat <- matrix(1:12,4,3)
xx <- mEdit(data = xMat)
xx$edit(2, 2, 0)
xx
xx$undo()
mEdit$help("undo")
stopifnot(all.equal(xx$data, xMat))

utils::str(xx) # show fields and names of methods

## A method to save the object
mEdit$methods(
     save = function(file) {
       'Save the current object on the file
        in R external object format.
       '
         base::save(.self, file = file)
     }
)

tf <- tempfile()
xx$save(tf)
## Don't show: 
load(tf)
unlink(tf)
stopifnot(identical(xx$data, .self$data))
## End(Don't show)

## Not run: 
##D ## Inheriting a reference class:  a matrix viewer
##D mv <- setRefClass("matrixViewer",
##D     fields = c("viewerDevice", "viewerFile"),
##D     contains = "mEdit",
##D     methods = list( view = function() {
##D         dd <- dev.cur(); dev.set(viewerDevice)
##D         devAskNewPage(FALSE)
##D         matplot(data, main = paste("After",length(edits),"edits"))
##D         dev.set(dd)},
##D         edit = # invoke previous method, then replot
##D           function(i, j, value) {
##D             callSuper(i, j, value)
##D             view()
##D           }))
##D 
##D ## initialize and finalize methods
##D mv$methods( initialize =
##D   function(file = "./matrixView.pdf", ...) {
##D     viewerFile <<- file
##D     pdf(viewerFile)
##D     viewerDevice <<- dev.cur()
##D     dev.set(dev.prev())
##D     callSuper(...)
##D   },
##D   finalize = function() {
##D     dev.off(viewerDevice)
##D   })
##D 
##D ## debugging an object: call browser() in method $edit()
##D xx$trace(edit, browser)
##D 
##D ## debugging all objects from class mEdit in method $undo()
##D mEdit$trace(undo, browser)
## End(Not run)
## Don't show: 
removeClass("mEdit")
resetGeneric("$")
resetGeneric("initialize")
## End(Don't show)
 



