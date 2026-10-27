RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: sink
### Title: Send R Output to a File
### Aliases: sink sink.number
### Keywords: file connection

### ** Examples

sink("sink-examp.txt")
i <- 1:10
outer(i, i)
sink()
## Don't show: 
unlink("sink-examp.txt")
## End(Don't show)
## No test: ##D 
##D ## capture all the output to a file.
##D zz <- file("all.Rout", open = "wt")
##D sink(zz)
##D sink(zz, type = "message")
##D try(log("a"))
##D ## revert output back to the console -- only then access the file!
##D sink(type = "message")
##D sink()
##D file.show("all.Rout", delete.file = TRUE)
## End(No test)


