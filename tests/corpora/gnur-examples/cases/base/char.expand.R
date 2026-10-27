RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: char.expand
### Title: Expand a String with Respect to a Target Table
### Aliases: char.expand
### Keywords: character

### ** Examples

locPars <- c("mean", "median", "mode")
char.expand("me", locPars, warning("Could not expand!"))
char.expand("mo", locPars)



