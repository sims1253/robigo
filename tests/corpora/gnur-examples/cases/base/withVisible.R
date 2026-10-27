RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: withVisible
### Title: Return both a Value and its Visibility
### Aliases: withVisible
### Keywords: programming

### ** Examples

x <- 1
withVisible(x <- 1) # *$visible is FALSE
x
withVisible(x)      # *$visible is TRUE

# Wrap the call in evalq() for special handling

df <- data.frame(a = 1:5, b = 1:5)
evalq(withVisible(a + b), envir = df)



