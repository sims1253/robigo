RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: serialize
### Title: Simple Serialization Interface
### Aliases: serialize unserialize
### Keywords: file connection

### ** Examples

x <- serialize(list(1,2,3), NULL)
unserialize(x)

## see also the examples for saveRDS



