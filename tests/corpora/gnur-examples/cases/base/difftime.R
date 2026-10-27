### Name: difftime
### Title: Time Intervals / Differences
### Aliases: difftime as.difftime *.difftime /.difftime Math.difftime
###   Ops.difftime Summary.difftime [.difftime [<-.difftime
###   as.double.difftime as.list.difftime c.difftime diff.difftime
###   format.difftime is.numeric.difftime length<-.difftime mean.difftime
###   print.difftime rep.difftime units.difftime units<-.difftime 'time
###   interval' summary.difftime
### Keywords: utilities chron

### ** Examples
## No test: 
##D (z <- Sys.time() - 3600)
##D Sys.time() - z                # just over 3600 seconds.
##D 
##D ## time interval between release days of R 1.2.2 and 1.2.3.
##D ISOdate(2001, 4, 26) - ISOdate(2001, 2, 26)
##D 
##D as.difftime(c("0:3:20", "11:23:15"))
##D as.difftime(c("3:20", "23:15", "2:"), format = "%H:%M") # 3rd gives NA
##D (z <- as.difftime(c(0,30,60), units = "mins"))
##D as.numeric(z, units = "secs")
##D as.numeric(z, units = "hours")
##D format(z)
## End(No test)


