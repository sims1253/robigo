RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: gzcon
### Title: (De)compress I/O Through Connections
### Aliases: gzcon
### Keywords: file connection

### ** Examples

## No test: 
##D ## Uncompress a data file from a URL
##D z <- gzcon(url("https://www.stats.ox.ac.uk/pub/datasets/csb/ch12.dat.gz"))
##D # read.table can only read from a text-mode connection.
##D raw <- textConnection(readLines(z))
##D close(z)
##D dat <- read.table(raw)
##D close(raw)
##D dat[1:4, ]
## End(No test)

## gzfile and gzcon can inter-work.
## Of course here one would use gzfile, but file() can be replaced by
## any other connection generator.
zzfil <- tempfile(fileext = ".gz")
zz <- gzfile(zzfil, "w")
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = zz, sep = "\n")
close(zz)
readLines(zz <- gzcon(file(zzfil, "rb")))
close(zz)
unlink(zzfil)

zzfil2 <- tempfile(fileext = ".gz")
zz <- gzcon(file(zzfil2, "wb"))
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = zz, sep = "\n")
close(zz)
readLines(zz <- gzfile(zzfil2))
close(zz)
unlink(zzfil2)



