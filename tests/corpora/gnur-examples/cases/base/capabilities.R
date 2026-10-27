RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: capabilities
### Title: Report Capabilities of this Build of R
### Aliases: capabilities
### Keywords: utilities

### ** Examples

capabilities()

if(!capabilities("ICU"))
   warning("ICU is not available")

## Does not call the internal X11-checking function:
capabilities(Xchk = FALSE)

## See also the examples for 'connections'.



