RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: prompt
### Title: Produce Prototype of an R Documentation File
### Aliases: prompt prompt.default prompt.data.frame promptImport
### Keywords: documentation

### ** Examples

require(graphics)
## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)
prompt(plot.default)
prompt(interactive, force.function = TRUE)
unlink("plot.default.Rd")
unlink("interactive.Rd")

prompt(women) # data.frame
unlink("women.Rd")

prompt(sunspots) # non-data.frame data
unlink("sunspots.Rd")

## Don't show: 
setwd(oldwd)
## End(Don't show)
## Not run: 
##D ## Create a help file for each function in the .GlobalEnv:
##D for(f in ls()) if(is.function(get(f))) prompt(name = f)
## End(Not run)




