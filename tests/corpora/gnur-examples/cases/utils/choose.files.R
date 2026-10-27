RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: choose.files
### Title: Choose a List of Files Interactively on MS Windows
### Aliases: choose.files Filters
### Keywords: file

### ** Examples

  if (interactive() && .Platform$OS.type == "windows")
       choose.files(filters = Filters[c("zip", "All"),])



