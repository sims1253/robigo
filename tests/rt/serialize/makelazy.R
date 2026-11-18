# makeLazy (builtin.c's do_makelazy) as lazyLoad uses it: each value of a
# database written with lazyLoadDBinsertValue bound as a promise of
# lazyLoadDBfetch, the key evaluated in eval.env and put in the
# expression's second place; and its errors.
msg <- function(expr) tryCatch({ force(expr); "no error" }, error = function(e) conditionMessage(e))
db <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-makelazy-", Sys.getpid(), ".rdb")
vals <- list(a = 1:3, b = c(x = "y"), f = function(z) z + 1, l = list(1, "two"))
keys <- lapply(vals, function(v) .Internal(lazyLoadDBinsertValue(v, db, FALSE, 1L, NULL)))
# (the keys' lengths are the compressed records': zlib-rs is not zlib)
print(sapply(keys, function(k) paste(typeof(k), length(k))))
e <- new.env()
datafile <- db
compressed <- TRUE
envhook <- NULL
.Internal(makeLazy(names(keys), keys, quote(lazyLoadDBfetch(key, datafile, compressed, envhook)), environment(), e))
print(sort(ls(e)))
print(e$a)
print(e$b)
print(e$f(1))
print(identical(e$l, vals$l))
# the key is an expression evaluated in eval.env
k <- keys$a
e2 <- new.env()
.Internal(makeLazy("a2", list(quote(k)), quote(lazyLoadDBfetch(key, datafile, compressed, envhook)), environment(), e2))
print(e2$a2)
.Internal(lazyLoadDBflush(db))
print(msg(.Internal(makeLazy(1, keys, quote(f(key)), environment(), e))))
print(msg(.Internal(makeLazy("a", keys, quote(f(key)), 1, e))))
print(msg(.Internal(makeLazy("a", keys, quote(f(key)), environment(), NULL))))
print(msg(.Internal(makeLazy("a", 1:3, quote(f(key)), environment(), e))))
print(file.remove(db))
