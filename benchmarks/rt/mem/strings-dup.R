# Memory benchmark (D10, reported only): 1e6 strings with 10 distinct values,
# made by coercion, with no use that needs their pointers. Peak RSS by
# scripts/rss.py.
x <- as.character(rep(1:10, 100000))
cat(length(x), x[1000000], "\n")
