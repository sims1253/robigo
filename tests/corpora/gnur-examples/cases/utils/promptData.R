### Name: promptData
### Title: Generate Outline Documentation for a Data Set
### Aliases: promptData
### Keywords: documentation

### ** Examples

## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)
promptData(sunspots)
unlink("sunspots.Rd")
## Don't show: 
setwd(oldwd)
## End(Don't show)



