RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: showNonASCII
### Title: Pick Out Non-ASCII Characters
### Aliases: showNonASCII showNonASCIIfile
### Keywords: utilities

### ** Examples

out <- c(
"fran\xE7ais: test of showNonASCII():",
"\\details{",
"   This is a good line",
"   This has an \xfcmlaut in it.",
"   OK again.",
"}")
f <- tempfile()
cat(out, file = f, sep = "\n")

## IGNORE_RDIFF_BEGIN
showNonASCIIfile(f)
## IGNORE_RDIFF_END
unlink(f)



