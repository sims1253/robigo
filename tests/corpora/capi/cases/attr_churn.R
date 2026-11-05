dyn.load("attr_churn.so")

# what C writes reaches the bound object
e <- new.env()
invisible(.Call("capi_churn_store", e, "w", 3L))
print(e$w)
invisible(.Call("capi_churn_setattr", e$w, "tag", "from C"))
print(attributes(e$w))
invisible(.Call("capi_churn_poke_name", e$w, 2L, "zz"))
print(names(e$w))
w <- e$w
invisible(.Call("capi_churn_touch", w))
print(identical(names(w), c("a1", "zz", "a3")))

# A routine that leaves the attributes alone makes no copies of them:
# calls on a large named vector inside a function (no return to top level)
# take no memory for them.
rss <- function() {
    s <- grep("^VmRSS:", readLines("/proc/self/status"), value = TRUE)
    as.numeric(gsub("[^0-9]", "", s)) / 1024
}
invisible(.Call("capi_churn_store", e, "v", 20000L))
calls <- function(n) {
    before <- rss()
    for (k in seq_len(n)) invisible(.Call("capi_churn_touch", e$v))
    rss() - before < 50
}
print(calls(300))
print(names(e$v)[1:3])
