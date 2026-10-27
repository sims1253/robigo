RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: list.files
### Title: List the Files in a Directory/Folder
### Aliases: list.files dir list.dirs
### Keywords: file

### ** Examples

list.files(R.home())
## Only files starting with a-l or r
## Note that a-l is locale-dependent, but using case-insensitive
## matching makes it unambiguous in English locales
dir("../..", pattern = "^[a-lr]", full.names = TRUE, ignore.case = TRUE)

list.dirs(R.home("doc"))
list.dirs(R.home("doc"), full.names = FALSE)



