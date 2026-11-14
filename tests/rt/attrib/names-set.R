# `names<-`: a value not a plain character vector goes through
# as.character; short values are padded with NA; pairlists and calls take
# tags
v <- 1:3
names(v) <- c("a", "b")
v
names(v) <- 1:3
v
names(v) <- c(1.5, 2, NA)
v
names(v) <- NULL
v
names(v) <- c(x = "p", y = "q", z = "r")
attributes(v)
k <- 1:2
names(k) <- c(NA, "b")
k
names(k)
l <- list(1, 2)
names(l) <- c("x", "y")
names(l)
p <- pairlist(1, 2)
names(p) <- c("u", "")
names(p)
names(p) <- NULL
names(p)
q <- quote(f(1, 2))
names(q) <- c("", "a", "b")
q
names(q) <- NULL
q
z <- 1:2
names(z) <- pairlist("a", 2)
z
e <- NULL
names(e) <- NULL
e
