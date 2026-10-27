RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: sum
### Title: Sum of Vector Elements
### Aliases: sum
### Keywords: arith

### ** Examples

## Pass a vector to sum, and it will add the elements together.
sum(1:5)

## Pass several numbers to sum, and it also adds the elements.
sum(1, 2, 3, 4, 5)

## In fact, you can pass vectors into several arguments, and everything gets added.
sum(1:2, 3:5)

## If there are missing values, the sum is unknown, i.e., also missing, ....
sum(1:5, NA)
## ... unless  we exclude missing values explicitly:
sum(1:5, NA, na.rm = TRUE)



