RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: jitter
### Title: 'Jitter' (Add Noise) to Numbers
### Aliases: jitter
### Keywords: dplot utilities

### ** Examples

round(jitter(c(rep(1, 3), rep(1.2, 4), rep(3, 3))), 3)
## These two 'fail' with S-plus 3.x:
jitter(rep(0, 7))
jitter(rep(10000, 5))



