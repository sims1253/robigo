RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: poisson.test
### Title: Exact Poisson tests
### Aliases: poisson.test
### Keywords: htest

### ** Examples

### These are paraphrased from data sets in the ISwR package

## SMR, Welsh Nickel workers
poisson.test(137, 24.19893)

## eba1977, compare Fredericia to other three cities for ages 55-59
poisson.test(c(11, 6+8+7), c(800, 1083+1050+878))



