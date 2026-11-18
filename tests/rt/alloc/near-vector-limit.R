# Near a vector limit, small requests that GNU R's collection cannot make
# room for end in GNU R's limit error, not "cannot allocate vector of
# size": AdjustHeapSize grows the vector heap only while what is in use,
# the request and R_MinVFree stay below the limit. The limit is 1 Mb past
# the trigger, and the requests fill all but 150,000 VECRECs of it (robigo
# before this case kept allocating to the limit, counting garbage).
g <- gc()
lim <- ceiling(g[2, 3] * 8 / 1048576) + 1
invisible(mem.maxVSize(lim))
n <- floor((lim * 1048576 / 8 - 150000 - g[2, 1]) / 100)
x <- vector("list", n)
for (i in seq_len(n)) x[[i]] <- numeric(100)
cat("all fit\n")
