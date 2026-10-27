RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Sys.which
### Title: Find Full Paths to Executables
### Aliases: Sys.which
### Keywords: utilities

### ** Examples

## the first two are likely to exist everywhere
## texi2dvi exists on most Unix-alikes but not under MiKTeX
Sys.which(c("ftp", "ping", "texi2dvi", "this-does-not-exist"))



