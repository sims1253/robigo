RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: dump
### Title: Text Representations of R Objects
### Aliases: dump
### Keywords: file connection

### ** Examples

x <- 1; y <- 1:10
fil <- tempfile(fileext=".Rdmped")
dump(ls(pattern = '^[xyz]'), fil)
print(.Last.value)
unlink(fil)



