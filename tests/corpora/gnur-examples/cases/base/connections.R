RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: connections
### Title: Functions to Manipulate Connections (Files, URLs, ...)
### Aliases: connections connection file clipboard pipe fifo gzfile unz
###   bzfile xzfile zstdfile url socketConnection socketAccept serverSocket
###   socketTimeout open open.connection isOpen isIncomplete close
###   close.connection flush flush.connection print.connection
###   summary.connection
### Keywords: file connection

### ** Examples

zzfil <- tempfile(fileext=".data")
zz <- file(zzfil, "w")  # open an output file connection
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = zz, sep = "\n")
cat("One more line\n", file = zz)
close(zz)
readLines(zzfil)
unlink(zzfil)

zzfil <- tempfile(fileext=".gz")
zz <- gzfile(zzfil, "w")  # compressed file
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = zz, sep = "\n")
close(zz)
readLines(zz <- gzfile(zzfil))
close(zz)
unlink(zzfil)
zz # an invalid connection

zzfil <- tempfile(fileext=".bz2")
zz <- bzfile(zzfil, "w")  # bzip2-ed file
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = zz, sep = "\n")
close(zz)
zz # print() method: invalid connection
print(readLines(zz <- bzfile(zzfil)))
close(zz)
unlink(zzfil)

## An example of a file open for reading and writing
Tpath <- tempfile("test")
Tfile <- file(Tpath, "w+")
c(isOpen(Tfile, "r"), isOpen(Tfile, "w")) # both TRUE
cat("abc\ndef\n", file = Tfile)
readLines(Tfile)
seek(Tfile, 0, rw = "r") # reset to beginning
readLines(Tfile)
cat("ghi\n", file = Tfile)
readLines(Tfile)
## IGNORE_RDIFF_BEGIN
Tfile # -> print() :  "valid" connection
close(Tfile)
Tfile # -> print() :  "invalid" connection
unlink(Tpath)
## IGNORE_RDIFF_END
## We can do the same thing with an anonymous file.
Tfile <- file()
cat("abc\ndef\n", file = Tfile)
readLines(Tfile)
close(Tfile)

## Not run: 
##D ## fifo example -- may hang even with OS support for fifos
##D if(capabilities("fifo")) {
##D   zzfil <- tempfile(fileext="-fifo")
##D   zz <- fifo(zzfil, "w+")
##D   writeLines("abc", zz)
##D   print(readLines(zz))
##D   close(zz)
##D   unlink(zzfil)
##D }
## End(Not run)
## No test: 
##D ## Unix examples of use of pipes
##D 
##D # read listing of current directory
##D readLines(pipe("ls -1"))
##D 
##D # remove trailing commas.  Suppose
## Don't show: ##D 
##D oldwd <- setwd(tempdir())
##D writeLines(c("450, 390, 467, 654,  30, 542, 334, 432, 421,",
##D "357, 497, 493, 550, 549, 467, 575, 578, 342,",
##D "446, 547, 534, 495, 979, 479"), "data2_")
## End(Don't show)
##D 
## Not run: 
##D ##D % cat data2_
##D ##D 450, 390, 467, 654,  30, 542, 334, 432, 421,
##D ##D 357, 497, 493, 550, 549, 467, 575, 578, 342,
##D ##D 446, 547, 534, 495, 979, 479
## End(Not run)
##D 
##D # Then read this by
##D scan(pipe("sed -e s/,$// data2_"), sep = ",")
## Don't show: 
##D unlink("data2_"); setwd(oldwd)
## End(Don't show)
##D 
##D 
##D # convert decimal point to comma in output: see also write.table
##D # both R strings and (probably) the shell need \ doubled
##D zzfil <- tempfile("outfile")
##D zz <- pipe(paste("sed s/\\\\./,/ >", zzfil), "w")
##D cat(format(round(stats::rnorm(48), 4)), fill = 70, file = zz)
##D close(zz)
##D file.show(zzfil, delete.file = TRUE)
## End(No test)
## Not run: 
##D ## example for a machine running a finger daemon
##D 
##D con <- socketConnection(port = 79, blocking = TRUE)
##D writeLines(paste0(system("whoami", intern = TRUE), "\r"), con)
##D gsub(" *$", "", readLines(con))
##D close(con)
## End(Not run)

## Not run: 
##D ## Two R processes communicating via non-blocking sockets
##D # R process 1
##D con1 <- socketConnection(port = 6011, server = TRUE)
##D writeLines(LETTERS, con1)
##D close(con1)
##D 
##D # R process 2
##D con2 <- socketConnection(Sys.info()["nodename"], port = 6011)
##D # as non-blocking, may need to loop for input
##D readLines(con2)
##D while(isIncomplete(con2)) {
##D    Sys.sleep(1)
##D    z <- readLines(con2)
##D    if(length(z)) print(z)
##D }
##D close(con2)
##D 
##D ## examples of use of encodings
##D # write a file in UTF-8
##D cat(x, file = (con <- file("foo", "w", encoding = "UTF-8"))); close(con)
##D # read a 'Windows Unicode' file
##D A <- read.table(con <- file("students", encoding = "UCS-2LE")); close(con)
## End(Not run)


