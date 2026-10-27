RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: expression
### Title: Unevaluated Expressions
### Aliases: expression is.expression as.expression as.expression.default
### Keywords: programming dplot

### ** Examples

length(ex1 <- expression(1 + 0:9)) # 1
ex1
eval(ex1) # 1:10

length(ex3 <- expression(u, 2, u + 0:9)) # 3
mode(ex3 [3])   # expression
mode(ex3[[3]])  # call
## but not all components are 'call's :
sapply(ex3, mode  ) #  name  numeric  call
sapply(ex3, typeof) # symbol  double  language
rm(ex3)



