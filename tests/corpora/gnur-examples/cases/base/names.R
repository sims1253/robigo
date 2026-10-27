### Name: names
### Title: The Names of an Object
### Aliases: names names.default names<- names<-.default
### Keywords: attribute

### ** Examples

# print the names attribute of the islands data set
names(islands)

# remove the names attribute
names(islands) <- NULL
islands
rm(islands) # remove the copy made

z <- list(a = 1, b = "c", c = 1:3)
names(z)
# change just the name of the third element.
names(z)[3] <- "c2"
z

z <- 1:3
names(z)
## assign just one name
names(z)[2] <- "b"
z

## Don't show: 
## "show" the equivalence claimed above:
  for(e in c(baseenv(), globalenv()))
  stopifnot(identical(names(e), ls(e, all.names=TRUE, sorted=FALSE)),
            identical(names(e), names(as.list(e, all.names=TRUE))))
## End(Don't show)



