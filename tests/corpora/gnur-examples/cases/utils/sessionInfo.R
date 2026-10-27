RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: sessionInfo
### Title: Collect Information About the Current R Session
### Aliases: sessionInfo osVersion toLatex.sessionInfo print.sessionInfo
### Keywords: misc

### ** Examples
## No test: 
##D sI <- sessionInfo()
##D sI
##D # The same, showing the RNGkind, but not the locale :
##D   print(sI, RNG = TRUE, locale = FALSE)
##D toLatex(sI, locale = FALSE) # shortest; possibly desirable at end of report
## End(No test)


