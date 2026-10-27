RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ftable.formula
### Title: Formula Notation for Flat Contingency Tables
### Aliases: ftable.formula
### Keywords: category

### ** Examples

Titanic
x <- ftable(Survived ~ ., data = Titanic)
x
ftable(Sex ~ Class + Age, data = x)



