### Name: load
### Title: Reload Saved Datasets
### Aliases: load
### Keywords: file

### ** Examples

## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)

## save all data
xx <- pi # to ensure there is some data
save(list = ls(all.names = TRUE), file= "all.rda")
rm(xx)

## restore the saved values to the current environment
local({
   load("all.rda")
   ls()
})

xx <- exp(1:3)
## restore the saved values to the user's workspace
load("all.rda") ## which is here *equivalent* to
## load("all.rda", .GlobalEnv)
## This however annihilates all objects in .GlobalEnv with the same names !
xx # no longer exp(1:3)
rm(xx)
attach("all.rda") # safer and will warn about masked objects w/ same name in .GlobalEnv
ls(pos = 2)
##  also typically need to cleanup the search path:
detach("file:all.rda")

## clean up (the example):
unlink("all.rda")
## Don't show: 
setwd(oldwd)
## End(Don't show)

## Not run: 
##D con <- url("http://some.where.net/R/data/example.rda")
##D ## print the value to see what objects were created.
##D print(load(con))
##D close(con) # url() always opens the connection
## End(Not run)


