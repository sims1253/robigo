RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: sortedXyData
### Title: Create a 'sortedXyData' Object
### Aliases: sortedXyData sortedXyData.default
### Keywords: manip

### ** Examples

DNase.2 <- DNase[ DNase$Run == "2", ]
sortedXyData( expression(log(conc)), expression(density), DNase.2 )



