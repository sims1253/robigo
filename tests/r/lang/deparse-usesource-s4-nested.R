# a srcref method that deparses an S4 object keeps the outer deparse's S4 parts
library(methods)
setClass("P", representation(x = "numeric"))
registerS3method("as.character", "nestsrc", function(x, ...) { deparse(new("P", x = 2)); "a  # src" })
e <- expression(a, b)
e[[2]] <- new("P", x = 1)
attr(e, "srcref") <- list(structure(1, class = "nestsrc"), NULL)
print(deparse(e, control = "useSource"))
