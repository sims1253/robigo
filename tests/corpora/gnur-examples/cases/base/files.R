RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: files
### Title: File Manipulation
### Aliases: files file.append file.copy file.create file.exists
###   file.remove file.rename file.symlink file.link
### Keywords: file

### ** Examples

## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)
cat("file A\n", file = "A")
cat("file B\n", file = "B")
file.append("A", "B")
file.create("A") # (trashing previous)
file.append("A", rep("B", 10))
if(interactive()) file.show("A") # -> the 10 lines from 'B'
file.copy("A", "C")
dir.create("tmp")
file.copy(c("A", "B"), "tmp")
list.files("tmp") # -> "A" and "B"
setwd("tmp")
file.remove("A") # the tmp/A file
file.symlink(file.path("..", c("A", "B")), ".")
                     # |--> (TRUE,FALSE) : ok for A but not B as it exists already
setwd("..")
unlink("tmp", recursive = TRUE)
file.remove("A", "B", "C")
## Don't show: 
setwd(oldwd)
## End(Don't show)



