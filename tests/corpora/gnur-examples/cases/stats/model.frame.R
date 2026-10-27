RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: model.frame
### Title: Extracting the Model Frame from a Formula or Fit
### Aliases: model.frame model.frame.default model.frame.lm model.frame.glm
###   model.frame.aovlist get_all_vars
### Keywords: models

### ** Examples

data.class(model.frame(dist ~ speed, data = cars))

## using a subset and an extra variable
model.frame(dist ~ speed, data = cars, subset = speed < 10, z = log(dist))

## get_all_vars(): new var.s are recycled (iff length matches: 50 = 2*25)
ncars <- get_all_vars(sqrt(dist) ~ I(speed/2), data = cars, newVar = 2:3)
stopifnot(is.data.frame(ncars),
          identical(cars, ncars[,names(cars)]),
          ncol(ncars) == ncol(cars) + 1)



