### Name: save
### Title: Save R Objects
### Aliases: save save.image
### Keywords: file

### ** Examples

## Don't show: 
oldwd <- setwd(tempdir()) # so examples do write there
## End(Don't show)
x <- stats::runif(20)
y <- list(a = 1, b = TRUE, c = "oops")
save(x, y, file = "xy.RData")
save.image() # creating ".RData" in current working directory
unlink("xy.RData")

# set save defaults using option:
options(save.defaults = list(ascii = TRUE, safe = FALSE))
save.image() # creating ".RData"
if(interactive()) withAutoprint({
   file.info(".RData")
   readLines(".RData", n = 7) # first 7 lines; first starts w/ "RDA"..
})
unlink(".RData")
## Don't show: 
setwd(oldwd)
## End(Don't show)



