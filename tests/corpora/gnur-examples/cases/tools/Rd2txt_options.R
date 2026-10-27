library(tools)
### Name: Rd2txt_options
### Title: Set Formatting Options for Text Help
### Aliases: Rd2txt_options
### Keywords: documentation

### ** Examples

saveOpts <- Rd2txt_options()
## No test: 
##D saveOpts
## End(No test)
Rd2txt_options(minIndent = 4)
Rd2txt_options()[["minIndent"]]
Rd2txt_options(saveOpts)
stopifnot(identical(Rd2txt_options(), saveOpts))



