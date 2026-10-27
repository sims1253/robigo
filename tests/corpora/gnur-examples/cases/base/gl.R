RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: gl
### Title: Generate Factor Levels
### Aliases: gl
### Keywords: category arith

### ** Examples

## First control, then treatment:
gl(2, 8, labels = c("Control", "Treat"))
## 20 alternating 1s and 2s
gl(2, 1, 20)
## alternating pairs of 1s and 2s
gl(2, 2, 20)



