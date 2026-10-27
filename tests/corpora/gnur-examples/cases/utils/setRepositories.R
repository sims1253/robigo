RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: setRepositories
### Title: Select Package Repositories
### Aliases: setRepositories R_REPOSITORIES R_BIOC_VERSION
### Keywords: utilities

### ** Examples
## Not run: 
##D setRepositories(addURLs =
##D                 c(CRANxtras = "https://www.stats.ox.ac.uk/pub/RWin"))
## End(Not run)
oldrepos <- setRepositories(name = c("CRAN", "R-Forge"))
## No test: 
##D getOption("repos")
## End(No test)
options(oldrepos) # restore



