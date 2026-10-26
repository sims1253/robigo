# Data frame manipulation: creation, column ops, subsetting, ordering,
# aggregation with tapply, split/lapply.
set.seed(42)
n <- 1e5
df <- data.frame(id = 1:n, g = sample(letters[1:20], n, TRUE), x = (1:n) %% 1000 / 10, y = sin(1:n))
df$z <- df$x * 2 + df$y
sub <- df[df$x > 50 & df$g %in% c("a", "b", "c"), ]
srt <- df[order(df$g, -df$x), c("id", "g", "x")]
m <- tapply(df$z, df$g, mean)
sp <- vapply(split(df$x, df$g), max, 0)
cat(nrow(sub), sum(sub$z), srt$id[1:3], sprintf("%.6f", sum(m)), sum(sp), "\n")
