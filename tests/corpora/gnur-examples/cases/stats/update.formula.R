RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: update.formula
### Title: Model Updating
### Aliases: update.formula
### Keywords: models

### ** Examples

update(y ~ x,    ~ . + x2) #> y ~ x + x2
update(y ~ x, log(.) ~ . ) #> log(y) ~ x
update(. ~ u+v, res  ~ . ) #> res ~ u + v



