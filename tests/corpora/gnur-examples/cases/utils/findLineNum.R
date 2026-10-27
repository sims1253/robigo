RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: findLineNum
### Title: Find the Location of a Line of Source Code, or Set a Breakpoint
###   There
### Aliases: findLineNum setBreakpoint
### Keywords: debugging

### ** Examples

## Not run: 
##D # Find what function was defined in the file mysource.R at line 100:
##D findLineNum("mysource.R#100")
##D 
##D # Set a breakpoint in both copies of that function, assuming one is in the
##D # same namespace as myfunction and the other is on the search path
##D setBreakpoint("mysource.R#100", envir = myfunction)
## End(Not run)



