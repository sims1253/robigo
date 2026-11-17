# vectorIndex through calls (isPairList: a call is a pairlist too), as
# zzz.R's as.data.frame.<class> deprecation code does: `[[` and `[[<-`
# with a vector index into a call's arguments, and into a call held in a
# list.
b <- quote({ f(1, g(2, "P")); h })
ii <- c(2L, 3L, 3L)
print(b[[ii]])
b[[ii]] <- "Q"
print(b)
print(b[[c(2L, 1L)]])
l <- list(a = quote(k(x, y)))
print(l[[c(1, 3)]])
l[[c(1, 2)]] <- quote(z)
print(l$a)
