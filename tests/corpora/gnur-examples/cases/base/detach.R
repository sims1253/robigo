### Name: detach
### Title: Detach Objects from the Search Path
### Aliases: detach
### Keywords: data

### ** Examples

require(splines) # package
detach(package:splines)
## or also
library(splines)
pkg <- "package:splines"
## Don't show: 
stopifnot(inherits(tryCatch(detach(pkg), error = function(.).),  "error"))
## End(Don't show)
detach(pkg, character.only = TRUE)

## careful: do not do this unless 'splines' is not already attached.
library(splines)
detach(2) # 'pos' used for 'name'

## an example of the name argument to attach
## and of detaching a database named by a character vector
attach_and_detach <- function(db, pos = 2)
{
   name <- deparse1(substitute(db))
   attach(db, pos = pos, name = name)
   print(search()[pos])
   detach(name, character.only = TRUE)
}
attach_and_detach(women, pos = 3)



