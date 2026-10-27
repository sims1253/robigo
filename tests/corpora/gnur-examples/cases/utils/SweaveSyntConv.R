RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: SweaveSyntConv
### Title: Convert Sweave Syntax
### Aliases: SweaveSyntConv
### Keywords: utilities

### ** Examples

testfile <- system.file("Sweave", "Sweave-test-1.Rnw", package = "utils")
## Don't show: 
oldwd <- setwd(tempdir())
## End(Don't show)

## convert the file to latex syntax
SweaveSyntConv(testfile, SweaveSyntaxLatex)

## and run it through Sweave
Sweave("Sweave-test-1.Stex")

## Don't show: 
 if(!interactive()) unlink("Sweave-test-1*") ; setwd(oldwd) 
## End(Don't show)



