### Name: library
### Title: Loading/Attaching and Listing of Packages
### Aliases: library require conflictRules .noGenerics format.libraryIQR
###   print.libraryIQR format.packageInfo print.packageInfo
### Keywords: data

### ** Examples

library()                   # list all available packages
library(lib.loc = .Library) # list all packages in the default library
## No test: 
##D library(help = splines)     # documentation on package 'splines'
## End(No test)
library(splines)            # attach package 'splines'
require(splines)            # the same
search()                    # "splines", too
detach("package:splines")

# if the package name is in a character vector, use
pkg <- "splines"
library(pkg, character.only = TRUE)
detach(pos = match(paste("package", pkg, sep = ":"), search()))

require(pkg, character.only = TRUE)
detach(pos = match(paste("package", pkg, sep = ":"), search()))

require(nonexistent)        # FALSE
## Not run: 
##D ## if you want to mask as little as possible, use
##D library(mypkg, pos = "package:base")
## End(Not run)


