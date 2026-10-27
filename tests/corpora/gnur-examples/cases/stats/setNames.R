RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: setNames
### Title: Set the Names in an Object
### Aliases: setNames
### Keywords: list

### ** Examples

setNames( 1:3, c("foo", "bar", "baz") )
# this is just a short form of
tmp <- 1:3
names(tmp) <-  c("foo", "bar", "baz")
tmp

## special case of character vector, using default
setNames(nm = c("First", "2nd"))



