RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Rtangle
### Title: R Driver for 'Stangle'
### Aliases: Rtangle RtangleSetup
### Keywords: utilities

### ** Examples

nmRnw <- "example-1.Rnw"
exfile <- system.file("Sweave", nmRnw, package = "utils")
## Create R source file
Stangle(exfile)
nmR <- sub("Rnw$", "R", nmRnw) # the (default) R output file name
if(interactive()) file.show("example-1.R")
## Don't show: 
file.rename("example-1.R", "example-1_def.R")
## End(Don't show)
## Smaller R source file with custom annotation:
my.Ann <- function(options, chunk, output) {
  cat("### chunk #", options$chunknr, ": ",
      if(!is.null(ol <- options$label)) ol else .RtangleCodeLabel(chunk),
      if(!options$eval) " (eval = FALSE)", "\n",
      file = output, sep = "")
}
Stangle(exfile, annotate = my.Ann)
if(interactive()) file.show("example-1.R")
## Don't show: 
file.rename("example-1.R", "example-1_myA.R")
## End(Don't show)
Stangle(exfile, annotate = my.Ann, drop.evalFALSE=TRUE)
if(interactive()) file.show("example-1.R")
## Don't show: 
file.rename("example-1.R", "example-1_myA-noF.R")
## End(Don't show)



