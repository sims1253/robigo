RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: selectSuperClasses
### Title: Super Classes (of Specific Kinds) of a Class
### Aliases: selectSuperClasses .selectSuperClasses
### Keywords: programming classes

### ** Examples

setClass("Root")
setClass("Base", contains = "Root", slots = c(length = "integer"))
setClass("A", contains = "Base", slots = c(x = "numeric"))
setClass("B", contains = "Base", slots = c(y = "character"))
setClass("C", contains = c("A", "B"))

extends("C")   #-->  "C"  "A" "B"  "Base" "Root"
selectSuperClasses("C") # "A" "B"
selectSuperClasses("C", directOnly=FALSE) # "A" "B"  "Base"  "Root"
selectSuperClasses("C", dropVirtual=TRUE, directOnly=FALSE)# ditto w/o "Root"



