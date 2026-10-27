RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: predict
### Title: Model Predictions
### Aliases: predict
### Keywords: methods

### ** Examples

## Don't show: 
old <- Sys.getlocale("LC_COLLATE")
       invisible(Sys.setlocale("LC_COLLATE", "C"))
## End(Don't show)
require(utils)

## All the "predict" methods found
## NB most of the methods in the standard packages are hidden.
## Output will depend on what namespaces are (or have been) loaded.
## IGNORE_RDIFF_BEGIN
for(fn in methods("predict"))
   try({
       f <- eval(substitute(getAnywhere(fn)$objs[[1]], list(fn = fn)))
       cat(fn, ":\n\t", deparse(args(f)), "\n")
       }, silent = TRUE)
## IGNORE_RDIFF_END
## Don't show: 
invisible(Sys.setlocale("LC_COLLATE", old))
## End(Don't show)



