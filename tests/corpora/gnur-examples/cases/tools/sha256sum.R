library(tools)
### Name: sha256sum
### Title: Compute SHA-256 Checksums
### Aliases: sha256sum
### Keywords: utilities

### ** Examples

as.vector(sha256sum(dir(R.home(), pattern = "^COPY", full.names = TRUE)))
sha256sum(bytes=raw())
sha256sum(bytes=charToRaw("abc"))



