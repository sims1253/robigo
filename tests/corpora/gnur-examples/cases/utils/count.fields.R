### Name: count.fields
### Title: Count the Number of Fields per Line
### Aliases: count.fields
### Keywords: file

### ** Examples

fil <- tempfile()
cat("NAME", "1:John", "2:Paul", file = fil, sep = "\n")
count.fields(fil, sep = ":")
unlink(fil)



