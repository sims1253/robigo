library(methods)
setClass("MetadataValue", slots = c(x = "numeric"))
setGeneric("metadataValue", function(x, y) standardGeneric("metadataValue"))
setMethod("metadataValue", c("MetadataValue", "ANY"), function(x, y) 1)
setMethod("metadataValue", c("ANY", "MetadataValue"), function(x, y) 2)
x <- new("MetadataValue", x = 1)
fenv <- environment(getGeneric("metadataValue"))
print(c(metadataValue(x, 0), metadataValue(0, x)))
original <- fenv$.SigArgs
# A list-to-list write does not advance Env::fn_version.
fenv$.SigArgs <- rev(original)
print(c(metadataValue(x, 0), metadataValue(0, x)))
fenv$.SigArgs <- original
print(c(metadataValue(x, 0), metadataValue(0, x)))
# A table replacement must be observed even when no function binding changes.
table <- fenv$.AllMTable
replacement <- new.env(parent = emptyenv())
for (name in ls(table, all.names = TRUE)) assign(name, get(name, table), replacement)
replacement$`MetadataValue#numeric` <- selectMethod("metadataValue", c("ANY", "MetadataValue"))
fenv$.AllMTable <- replacement
print(metadataValue(x, 0))
fenv$.AllMTable <- table
print(metadataValue(x, 0))
# Integer-to-integer metadata writes must also be observed.
fenv$.AllMTable$MetadataValue <- selectMethod("metadataValue", c("MetadataValue", "ANY"))
fenv$.SigLength <- 1L
print(metadataValue(x, x))
fenv$.SigLength <- 2L
print(metadataValue(0, x))
