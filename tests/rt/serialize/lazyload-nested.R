# Two fetches from a lazy-load database, one inside the other: the outer
# record's environment is a persistent name, and the restore hook fetches
# it, a second compressed record, while the outer one is being read
# (R_decompress1's inflater is used once per record, the inner fetch's
# while the outer's unserialize runs). Then the same again, and a third
# fetch after both.
path <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/robigo-rt-lazynest-", Sys.getpid(), ".rdb")
e <- new.env()
assign("v", 42, envir = e)
inner <- .Internal(lazyLoadDBinsertValue(as.list(e), path, FALSE, TRUE, NULL))
outer <- .Internal(lazyLoadDBinsertValue(list(x = 1:3, env = e, y = "after"), path, FALSE, TRUE,
                                         function(x) if (is.environment(x)) "env::1" else NULL))
.Internal(lazyLoadDBflush(path))
fetches <- 0L
hook <- function(n) {
    fetches <<- fetches + 1L
    vals <- lazyLoadDBfetch(inner, path, TRUE, NULL)
    r <- new.env()
    for (nm in names(vals)) assign(nm, vals[[nm]], envir = r)
    r
}
got <- lazyLoadDBfetch(outer, path, TRUE, hook)
print(got$x)
print(get("v", envir = got$env))
print(got$y)
print(fetches)
.Internal(lazyLoadDBflush(path))
again <- lazyLoadDBfetch(outer, path, TRUE, hook)
print(get("v", envir = again$env))
print(fetches)
print(lazyLoadDBfetch(inner, path, TRUE, NULL))
print(file.remove(path))
