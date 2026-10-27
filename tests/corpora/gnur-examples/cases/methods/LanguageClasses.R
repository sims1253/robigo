RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(methods)
### Name: language-class
### Title: Classes to Represent Unevaluated Language Objects
### Aliases: language-class (-class <--class call-class for-class if-class
###   repeat-class while-class name-class {-class
### Keywords: classes

### ** Examples

showClass("language")

is( quote(sin(x)) ) # "call"  "language"

(ff <- new("if"))  ; is(ff) # "if" "language"
(ff <- new("for")) ; is(ff) # "for" "language"



