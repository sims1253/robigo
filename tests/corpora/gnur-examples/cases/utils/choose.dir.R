RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: choose.dir
### Title: Choose a Folder Interactively on MS Windows
### Aliases: choose.dir
### Keywords: file

### ** Examples

  if (interactive() && .Platform$OS.type == "windows")
        choose.dir(getwd(), "Choose a suitable folder")



