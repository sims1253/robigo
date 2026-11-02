# envir.c:do_envprofile — a hashed environment's table: size, nchains and
# per-chain counts (a fresh table has 29 buckets); NULL for an unhashed
# environment, and only length(counts) is asserted in the help page.
eh <- new.env(hash = TRUE, size = NA)
str(env.profile(eh))
e2 <- new.env()
print(env.profile(e2))
e3 <- new.env(hash = TRUE, size = 5)
for (i in 1:40) assign(paste0("v", i), i, envir = e3)
p <- env.profile(e3)
with(p, stopifnot(size == length(counts), nchains <= size, sum(counts) == 40L))
print(p$nchains > 0L)
e4 <- new.env(parent = baseenv())
print(is.null(env.profile(e4)))
