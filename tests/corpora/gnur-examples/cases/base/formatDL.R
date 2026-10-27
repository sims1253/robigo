RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: formatDL
### Title: Format Description Lists
### Aliases: formatDL
### Keywords: print

### ** Examples

## Provide a nice summary of the numerical characteristics of the
## machine R is running on:
writeLines(formatDL(unlist(.Machine)))
## Inspect Sys.getenv() results in "list" style (by default, these are
## printed in "table" style):
## No test: 
##D writeLines(formatDL(Sys.getenv(), style = "list"))
## End(No test)



