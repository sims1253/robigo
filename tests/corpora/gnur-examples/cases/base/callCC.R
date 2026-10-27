RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: callCC
### Title: Call With Current Continuation
### Aliases: callCC
### Keywords: programming

### ** Examples

# The following all return the value 1
callCC(function(k) 1)
callCC(function(k) k(1))
callCC(function(k) {k(1); 2})
callCC(function(k) repeat k(1))



