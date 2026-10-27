### Name: readLines
### Title: Read Text Lines from a Connection
### Aliases: readLines
### Keywords: file connection

### ** Examples

fil <- tempfile(fileext = ".data")
cat("TITLE extra line", "2 3 5 7", "", "11 13 17", file = fil,
    sep = "\n")
readLines(fil, n = -1)
unlink(fil) # tidy up

## difference in blocking
fil <- tempfile("test")
cat("123\nabc", file = fil)
## IGNORE_RDIFF_BEGIN
readLines(fil) # line with a warning
## IGNORE_RDIFF_END

con <- file(fil, "r", blocking = FALSE)
readLines(con) # "123"
cat(" def\n", file = fil, append = TRUE)
readLines(con) # gets both
close(con)

unlink(fil) # tidy up

## Not run: 
##D # read a 'Windows Unicode' file
##D A <- readLines(con <- file("Unicode.txt", encoding = "UCS-2LE"))
##D close(con)
##D unique(Encoding(A)) # will most likely be UTF-8
## End(Not run)


