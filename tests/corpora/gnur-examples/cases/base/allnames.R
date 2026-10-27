RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: all.names
### Title: Find All Names in an Expression
### Aliases: all.names all.vars
### Keywords: programming

### ** Examples

all.names(expression(sin(x+y)))
all.names(quote(sin(x+y))) # or a call
all.vars(expression(sin(x+y)))



