### Name: promptPackage
### Title: Generate a Shell for Documentation of a Package
### Aliases: promptPackage
### Keywords: documentation

### ** Examples

filename <- tempfile()
## IGNORE_RDIFF_BEGIN
promptPackage("utils", filename = filename)
## IGNORE_RDIFF_END
file.show(filename)
unlink(filename)



