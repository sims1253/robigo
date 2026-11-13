# new.env(hash = FALSE): every binding outside a shape, listed newest first;
# rm then reinsertion moves to the front; as.list and mget follow ls.
e <- new.env(hash = FALSE)
for (n in c("p1", "p2", "p3", "p4", "p5")) assign(n, 1, envir = e)
print(ls(e, sorted = FALSE))
rm("p2", envir = e)
assign("p2", 2, envir = e)
print(ls(e, sorted = FALSE))
print(names(as.list(e)))
print(unlist(mget(c("p2", "p5"), envir = e)))
print(exists("p2", envir = e, inherits = FALSE))
print(env.profile(e))
l <- list2env(list(q = 1, w = 2, e = 3, r = 4))
print(ls(l, sorted = FALSE))
