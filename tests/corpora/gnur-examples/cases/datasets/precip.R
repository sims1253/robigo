### Name: precip
### Title: Annual Precipitation in US Cities
### Aliases: precip
### Keywords: datasets

### ** Examples

require(graphics)
dotchart(precip[order(precip)], main = "precip data")
title(sub = "Average annual precipitation (in.)")

## Old ("wrong") version of dataset (just name change):
precip.O <- local({
   p <- precip; names(p)[names(p) == "Cincinnati"] <- "Cincinati" ; p })
stopifnot(all(precip == precip.O),
	  match("Cincinnati", names(precip)) == 46,
	  identical(names(precip)[-46], names(precip.O)[-46]))



