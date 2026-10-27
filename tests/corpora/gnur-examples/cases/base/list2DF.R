RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: list2DF
### Title: Create Data Frame From List
### Aliases: list2DF
### Keywords: classes

### ** Examples

## Create a data frame holding a list of character vectors and the
## corresponding lengths:
x <- list(character(), "A", c("B", "C"))
n <- lengths(x)
list2DF(list(x = x, n = n))

## Create data frames with no variables and the desired number of rows:
list2DF()
list2DF(nrow = 3L)



