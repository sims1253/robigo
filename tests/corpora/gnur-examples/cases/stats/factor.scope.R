RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: factor.scope
### Title: Compute Allowed Changes in Adding to or Dropping from a Formula
### Aliases: add.scope drop.scope factor.scope
### Keywords: models

### ** Examples

add.scope( ~ a + b + c + a:b,  ~ (a + b + c)^3)
# [1] "a:c" "b:c"
drop.scope( ~ a + b + c + a:b)
# [1] "c"   "a:b"



