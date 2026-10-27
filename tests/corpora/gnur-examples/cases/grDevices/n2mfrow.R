RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: n2mfrow
### Title: Compute Default 'mfrow' From Number of Plots
### Aliases: n2mfrow
### Keywords: dplot utilities

### ** Examples

require(graphics)

n2mfrow(8) # 3 x 3

n <- 5 ; x <- seq(-2, 2, length.out = 51)
## suppose now that 'n' is not known {inside function}
op <- par(mfrow = n2mfrow(n))
for (j in 1:n)
   plot(x, x^j, main = substitute(x^ exp, list(exp = j)), type = "l",
   col = "blue")

sapply(1:14, n2mfrow)
sapply(1:14, n2mfrow, asp=16/9)



