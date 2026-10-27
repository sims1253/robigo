### Name: txtProgressBar
### Title: Text Progress Bar
### Aliases: txtProgressBar getTxtProgressBar setTxtProgressBar
###   close.txtProgressBar
### Keywords: utilities

### ** Examples
## No test: 
##D  # slow
##D testit <- function(x = sort(runif(20)), ...)
##D {
##D     pb <- txtProgressBar(...)
##D     for(i in c(0, x, 1)) {Sys.sleep(0.5); setTxtProgressBar(pb, i)}
##D     Sys.sleep(1)
##D     close(pb)
##D }
##D testit()
##D testit(runif(10))
##D testit(style = 3)
##D testit(char=' \u27a4')
## End(No test)


