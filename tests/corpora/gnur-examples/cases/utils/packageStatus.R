### Name: packageStatus
### Title: Package Management Tools
### Aliases: packageStatus print.packageStatus summary.packageStatus
###   update.packageStatus upgrade.packageStatus
### Keywords: utilities

### ** Examples

## No test: 
##D x <- packageStatus(repositories = contrib.url(findCRANmirror("web")))
##D print(x)
##D summary(x)
## End(No test)
## Not run: 
##D upgrade(x)
##D x <- update(x)
##D print(x)
## End(Not run)


