RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sweave
### Title: Automatic Generation of Reports
### Aliases: Sweave Stangle SweaveSyntaxLatex SweaveSyntaxNoweb
### Keywords: utilities

### ** Examples

testfile <- system.file("Sweave", "Sweave-test-1.Rnw", package = "utils")
## Don't show: 
oldwd <- setwd(tempdir()) # so we will write only to a temp directory
## End(Don't show)

## enforce par(ask = FALSE)
options(device.ask.default = FALSE)

## create a LaTeX file - in the current working directory, getwd():
Sweave(testfile)

## This can be compiled to PDF by
## tools::texi2pdf("Sweave-test-1.tex")

## or outside R by
##
## 	R CMD texi2pdf Sweave-test-1.tex
## on Unix-alikes which sets the appropriate TEXINPUTS path.
##
## On Windows,
##      Rcmd texify --pdf Sweave-test-1.tex
## if MiKTeX is available.

## create an R source file from the code chunks
Stangle(testfile)
## which can be sourced, e.g.
source("Sweave-test-1.R")

## Don't show: 
if(!interactive()) unlink("Sweave-test-1*")
setwd(oldwd)
## End(Don't show)



