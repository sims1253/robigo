library(tools)
### Name: R
### Title: Call Function in Inferior R Process
### Aliases: R

### ** Examples

## Compute cos(0) in an inferior R process.
## By default, only return the value of the function call.
R(cos, list(0))
## If 'drop = FALSE', we also get status, stdout and stderr.
## IGNORE_RDIFF_BEGIN
R(cos, list(0), drop = FALSE)
## IGNORE_RDIFF_END
    

## A call giving an error:
(e <- tryCatch(R(stop, list("FOOBAR")), error = identity))
## The inferior R process ran successfully:
e$status
## The function call gave an error:
e$value



