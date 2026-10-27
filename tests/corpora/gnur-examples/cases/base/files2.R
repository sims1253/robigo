RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: files2
### Title: Manipulation of Directories and File Permissions
### Aliases: dir.create dir.exists Sys.chmod Sys.umask umask
### Keywords: file

### ** Examples
## Not run: 
##D ## Fix up maximal allowed permissions in a file tree
##D Sys.chmod(list.dirs("."), "777")
##D f <- list.files(".", all.files = TRUE, full.names = TRUE, recursive = TRUE)
##D Sys.chmod(f, (file.mode(f) | "664"))
## End(Not run)


