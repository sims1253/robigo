RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: Rd2HTML
### Title: Rd Converters
### Aliases: Rd2txt Rd2HTML Rd2ex Rd2latex
### Keywords: documentation

### ** Examples
## No test: ##D 
##D ## Simulate rendering of this (installed) page in HTML and text format
##D Rd <- Rd_db("tools")[["Rd2HTML.Rd"]]
##D 
##D outfile <- tempfile(fileext = ".html")
##D Rd2HTML(Rd, outfile, package = "tools") |> browseURL()
##D 
##D outfile <- tempfile(fileext = ".txt")
##D Rd2txt(Rd, outfile, package = "tools") |> file.show()
## End(No test)


