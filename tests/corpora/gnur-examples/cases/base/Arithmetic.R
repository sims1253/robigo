RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Arithmetic
### Title: Arithmetic Operators
### Aliases: + - * ** / ^ %% %/% Arithmetic
### Keywords: arith

### ** Examples

x <- -1:12
x + 1
2 * x + 3
x %%  3 # is periodic  2 0  1  2 0  1 ...
x %% -3 #  (ditto)    -1 0 -2 -1 0 -2 ...
x %/% 5
x %% Inf # now is defined by limit (gave NaN in earlier versions of R)
## IGNORE_RDIFF_BEGIN
## Illustrating PR#18677, see above
1 %/% print(0.2, digits=19)
## IGNORE_RDIFF_END



