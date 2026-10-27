### Name: inheritedSlotNames
### Title: Names of Slots Inherited From a Super Class
### Aliases: inheritedSlotNames
### Keywords: classes methods

### ** Examples

.srch <- search()
library(stats4)
inheritedSlotNames("mle")

if(require("Matrix", quietly = TRUE)) withAutoprint({
  inheritedSlotNames("Matrix")       # NULL
  ## whereas
  inheritedSlotNames("sparseMatrix") # --> Dim & Dimnames
  ##  i.e. inherited from "Matrix" class
  cl <- getClass("dgCMatrix")        # six slots, etc
  inheritedSlotNames(cl) # *all* six slots are inherited
})
## Not run: 
##D 
##D 
##D ## detach package we've attached above:
##D for(n in rev(which(is.na(match(search(), .srch)))))
##D    try( detach(pos = n) )
## End(Not run)



