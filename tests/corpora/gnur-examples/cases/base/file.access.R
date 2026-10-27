RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: file.access
### Title: Ascertain File Accessibility
### Aliases: file.access
### Keywords: file

### ** Examples

fa <- file.access(dir("."))
## IGNORE_RDIFF_BEGIN
table(fa) # count successes & failures

## IGNORE_RDIFF_END



