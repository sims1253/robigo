df <- data.frame(x = 1:3, y = c("a", "b", "c"), z = c(1.5, 2.25, 3))
df
dim(df)
nrow(df)
names(df)
df$x
df[["y"]]
df[2, ]
df[df$x > 1, "z"]
summary(df$z)
str_df <- data.frame(name = c("alice", "bob"), score = c(90.5, 85), stringsAsFactors = TRUE)
str_df
levels(str_df$name)
data.frame(a = integer(0))
data.frame(n = c(10, 200, 3000), lbl = c("short", "a longer label", NA))
cbind(df, w = 3:1)
rbind(df, data.frame(x = 4L, y = "d", z = 4.5))
as.matrix(data.frame(p = 1:2, q = 3:4))
t(data.frame(p = 1:2, q = 3:4))
lapply(df, class)
sapply(df, is.numeric)
colMeans(df[, c("x", "z")])
f <- factor(c("lo", "hi", "lo", "mid"), levels = c("lo", "mid", "hi"))
f
table(f)
print(df, row.names = FALSE)
