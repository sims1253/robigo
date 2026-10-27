library(tools)
### Name: checkRd
### Title: Check an Rd Object
### Aliases: checkRd
### Keywords: utilities documentation

### ** Examples

## parsed Rd from the installed version of _this_ help file
rd <- Rd_db("tools")[["checkRd.Rd"]]
## No test: 
##D rd
## End(No test)
stopifnot(length(checkRd(rd)) == 0)  # there should be no issues

## make up \tabular issues
bad <- r"(\name{bad}\title{bad}\description{\tabular{p}{1 \tab 2}})"
(res <- checkRd(parse_Rd(textConnection(bad))))
stopifnot(length(res) > 0)



