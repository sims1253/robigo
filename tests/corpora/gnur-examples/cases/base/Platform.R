RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: .Platform
### Title: Platform Specific Variables
### Aliases: .Platform
### Keywords: file utilities

### ** Examples

## Note: this can be done in a system-independent way by dir.exists()
if(.Platform$OS.type == "unix") {
   system.test <- function(...) system(paste("test", ...)) == 0L
   dir.exists2 <- function(dir)
       sapply(dir, function(d) system.test("-d", d))
   dir.exists2(c(R.home(), "/tmp", "~", "/NO")) # > T T T F
}



