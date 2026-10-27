RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: count.fields
### Title: Count the Number of Fields per Line
### Aliases: count.fields
### Keywords: file

### ** Examples

fil <- tempfile()
cat("NAME", "1:John", "2:Paul", file = fil, sep = "\n")
count.fields(fil, sep = ":")
unlink(fil)



