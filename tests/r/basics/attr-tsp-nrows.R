# tsp<- takes n from nrows (util.c:nrows): a data frame is a list, so
# n is its LENGTH (the number of columns, it has no dim attribute) and
# the tsp validity check runs against that; language objects and
# symbols error "object is not a matrix"; expression vectors are
# vectors and take the tsp.
df <- data.frame(a = 1:3, b = 1:3)
print(try(attr(df, "tsp") <- c(1, 3, 1), silent = TRUE))
df3 <- data.frame(a = 1:3, b = 1:3, c = 1:3)
print(try(attr(df3, "tsp") <- c(1, 3, 1), silent = TRUE))
print(attr(df3, "tsp"))
lang <- quote(somecall)
print(try(attr(lang, "tsp") <- c(1, 3, 1), silent = TRUE))
expr <- expression(1, 2, 3)
print(try(attr(expr, "tsp") <- c(1, 3, 1), silent = TRUE))
print(attr(expr, "tsp"))
# a dim attribute wins over LENGTH, like any vector's
m <- matrix(1:6, 2)
print(try(attr(m, "tsp") <- c(1, 2, 1), silent = TRUE))
print(attr(m, "tsp"))
