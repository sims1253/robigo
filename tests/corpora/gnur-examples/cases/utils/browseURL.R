RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: browseURL
### Title: Load URL into an HTML Browser
### Aliases: browseURL
### Keywords: file

### ** Examples

## Not run: 
##D ## for KDE users who want to open files in a new tab
##D options(browser = "kfmclient newTab")
##D 
##D browseURL("https://www.r-project.org")
##D 
##D ## On Windows-only, something like
##D browseURL("file://d:/R/R-2.5.1/doc/html/index.html",
##D           browser = "C:/Program Files/Mozilla Firefox/firefox.exe")
## End(Not run)


