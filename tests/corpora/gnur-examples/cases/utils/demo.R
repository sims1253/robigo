### Name: demo
### Title: Demonstrations of R Functionality
### Aliases: demo
### Keywords: documentation utilities

### ** Examples

demo() # for attached packages

## All available demos:
demo(package = .packages(all.available = TRUE))

## No test: 
##D ## Display a demo, pausing between pages
##D demo(lm.glm, package = "stats", ask = TRUE)
##D 
##D ## Display it without pausing
##D demo(lm.glm, package = "stats", ask = FALSE)
## End(No test)

## Not run: 
##D  ch <- "scoping"
##D  demo(ch, character = TRUE)
## End(Not run)

## Find the location of a demo
system.file("demo", "lm.glm.R", package = "stats")



