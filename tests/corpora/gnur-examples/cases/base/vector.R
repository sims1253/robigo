RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: vector
### Title: Vectors - Creation, Coercion, etc
### Aliases: vector as.vector as.vector.data.frame as.vector.factor
###   as.vector.POSIXlt is.vector atomic
### Keywords: classes

### ** Examples

df <- data.frame(x = 1:3, y = 5:7)
## Error:
try(as.vector(data.frame(x = 1:3, y = 5:7), mode = "numeric"))

x <- c(a = 1, b = 2)
is.vector(x)
as.vector(x)
all.equal(x, as.vector(x)) ## FALSE


###-- All the following are TRUE:
is.list(df)
! is.vector(df)
! is.vector(df, mode = "list")

is.vector(list(), mode = "list")



