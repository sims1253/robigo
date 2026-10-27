RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: write.table
### Title: Data Output
### Aliases: write.table write.csv write.csv2
### Keywords: print file

### ** Examples

x <- data.frame(a = "a \" quote", b = pi)
tf <- tempfile(fileext = ".csv")

## To write a CSV file for input to Excel one might use
write.table(x, file = tf, sep = ",", col.names = NA,
            qmethod = "double")
file.show(tf)
## and to read this file back into R one needs
read.table(tf, header = TRUE, sep = ",", row.names = 1)
## NB: you do need to specify a separator if qmethod = "double".

### Alternatively
write.csv(x, file = tf)
read.csv(tf, row.names = 1)
## or without row names
write.csv(x, file = tf, row.names = FALSE)
read.csv(tf)

## Not run: 
##D ## To write a file in Mac Roman for simple use in Mac Excel 2004/8
##D write.csv(x, file = "foo.csv", fileEncoding = "macroman")
##D ## or for Windows Excel 2007/10
##D write.csv(x, file = "foo.csv", fileEncoding = "UTF-16LE")
## End(Not run)


