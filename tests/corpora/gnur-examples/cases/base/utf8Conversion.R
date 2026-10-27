RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: utf8Conversion
### Title: Convert Integer Vectors to or from UTF-8-encoded Character
###   Vectors
### Aliases: utf8ToInt intToUtf8 Unicode 'code point'
### Keywords: character utilities

### ** Examples
## No test: 
##D ## will only display in some locales and fonts
##D intToUtf8(0x03B2L) # Greek beta
## End(No test)
utf8ToInt("bi\u00dfchen")
utf8ToInt("\xfa\xb4\xbf\xbf\x9f")

## A valid UTF-16 surrogate pair (for U+10437)
x <- c(0xD801, 0xDC37)
intToUtf8(x)
intToUtf8(x, TRUE)
(xx <- intToUtf8(x, , TRUE)) # will only display in some locales and fonts
charToRaw(xx)

## No test: 
##D ## An example of how surrogate pairs might occur
##D x <- "\U10437"
##D charToRaw(x)
##D foo <- tempfile()
##D writeLines(x, file(foo, encoding = "UTF-16LE"))
##D ## next two are OS-specific, but are mandated by POSIX
##D system(paste("od -x", foo)) # 2-byte units, correct on little-endian platforms
##D system(paste("od -t x1", foo)) # single bytes as hex
##D y <- readBin(foo, "integer", 2, 2, FALSE, endian = "little")
##D sprintf("%X", y)
##D intToUtf8(y, , TRUE)
## End(No test)



