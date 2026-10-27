RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Trig
### Title: Trigonometric Functions
### Aliases: Trig cos sin tan acos arccos asin arcsin atan arctan atan2
###   cospi sinpi tanpi
### Keywords: math

### ** Examples

x <- seq(-3, 7, by = 1/8)
tx <- cbind(x, cos(pi*x), cospi(x), sin(pi*x), sinpi(x),
               tan(pi*x), tanpi(x), deparse.level=2)
op <- options(digits = 4, width = 90) # for nice formatting
head(tx)
tx[ (x %% 1) %in% c(0, 0.5) ,]
options(op)



