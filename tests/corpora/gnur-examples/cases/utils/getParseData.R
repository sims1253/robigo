### Name: getParseData
### Title: Get Detailed Parse Information from Object
### Aliases: getParseData getParseText
### Keywords: utilities

### ** Examples

fn <- function(x) {
  x + 1 # A comment, kept as part of the source
}

d <- getParseData(fn)
if (!is.null(d)) {
  plus <- which(d$token == "'+'")
  sum <- d$parent[plus]
  print(d[as.character(sum),])
  print(getParseText(d, sum))
}



