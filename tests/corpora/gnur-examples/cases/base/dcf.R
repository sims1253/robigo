RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: dcf
### Title: Read and Write Data in DCF Format
### Aliases: read.dcf write.dcf
### Keywords: print file

### ** Examples
## IGNORE_RDIFF_BEGIN
## Create a reduced version of the DESCRIPTION file in package 'splines'
x <- read.dcf(file = system.file("DESCRIPTION", package = "splines"),
              fields = c("Package", "Version", "Title"))
write.dcf(x)
## IGNORE_RDIFF_END
## No test: 
##D ## An online DCF file with multiple records
##D con <- url("https://cran.r-project.org/src/contrib/PACKAGES")
##D y <- read.dcf(con, all = TRUE)
##D close(con)
##D utils::str(y)
## End(No test)


