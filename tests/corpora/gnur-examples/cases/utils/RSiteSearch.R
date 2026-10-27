RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: RSiteSearch
### Title: Search for Key Words or Phrases in Documentation
### Aliases: RSiteSearch
### Keywords: utilities documentation

### ** Examples

## No test: 
##D # need Internet connection
##D ## for phrase searching you may use (escaped) double quotes or brackets
##D RSiteSearch("{logistic regression} \"glm object\"")
##D RSiteSearch('"logistic regression"')
##D 
##D ## Search in vignettes and help files of R base packages
##D ## store the query string:
##D fullquery <- RSiteSearch("lattice", restrict = c("vignettes","Rfunctions"))
##D fullquery # a string of 112 characters
## End(No test)



