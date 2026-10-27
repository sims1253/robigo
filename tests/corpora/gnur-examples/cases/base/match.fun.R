### Name: match.fun
### Title: Extract a Function Specified by Name
### Aliases: match.fun
### Keywords: programming

### ** Examples

# Same as get("*"):
match.fun("*")
# Overwrite outer with a vector
outer <- 1:5
try(match.fun(outer, descend = FALSE)) #-> Error:  not a function
match.fun(outer) # finds it anyway
is.function(match.fun("outer")) # as well



