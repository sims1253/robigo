RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.readlink
### Title: Read File Symbolic Links
### Aliases: Sys.readlink
### Keywords: file

### ** Examples

##' To check if files (incl. directories) are symbolic links:
is.symlink <- function(paths) isTRUE(nzchar(Sys.readlink(paths), keepNA=TRUE))
## will return all FALSE when the platform has no `readlink` system call.
is.symlink("/foo/bar")



