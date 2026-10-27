RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
library(tools)
### Name: md5sum
### Title: Compute MD5 Checksums
### Aliases: md5sum
### Keywords: utilities

### ** Examples

as.vector(md5sum(dir(R.home(), pattern = "^COPY", full.names = TRUE)))
md5sum(bytes=raw())
md5sum(bytes=charToRaw("abc"))



