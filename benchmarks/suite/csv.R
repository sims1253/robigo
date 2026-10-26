# Write and read a CSV file of 50k rows.
set.seed(3)
n <- 50000
df <- data.frame(a = 1:n, b = (1:n) / 3, c = paste0("s", (1:n) %% 101), d = (1:n) %% 2 == 0)
f <- tempfile(fileext = ".csv")
write.csv(df, f, row.names = FALSE)
back <- read.csv(f)
cat(nrow(back), sum(back$a), sprintf("%.6f", sum(back$b)), length(unique(back$c)), sum(back$d), "\n")
