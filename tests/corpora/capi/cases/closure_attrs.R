dyn.load("attr_sync.so")

# A closure's attributes are the object's: C reads what R set and R sees
# what C wrote, across calls.
f <- function(x) x
attr(f, "tag") <- 1:3
print(.Call("capi_attr_read", f, "tag"))
.Call("capi_attr_set", f, "from_c", "yes")
g <- f
print(attr(f, "from_c"))
print(attr(g, "from_c"))
print(.Call("capi_attr_unrelated"))
print(attr(f, "from_c"))
attr(f, "tag") <- 4:6
print(.Call("capi_attr_read", f, "tag"))
print(attr(g, "tag"))
.Call("capi_attr_set", f, "tag", NULL)
print(names(attributes(f)))
print(.Call("capi_attr_read", f, "tag"))

# Closures C has seen keep their attributes across later calls without
# copying them: a loop of unrelated calls inside a function (no return to
# top level) takes no memory for them (#302: ggplot2's S7 classes and
# generics).
rss <- function() {
    s <- grep("^VmRSS:", readLines("/proc/self/status"), value = TRUE)
    as.numeric(gsub("[^0-9]", "", s)) / 1024
}
fs <- lapply(1:50, function(i) {
    h <- function() i
    attr(h, "data") <- as.numeric(seq_len(1e4)) + i
    h
})
for (h in fs) invisible(.Call("capi_attr_touch", h))
calls <- function(n) {
    before <- rss()
    for (k in seq_len(n)) invisible(.Call("capi_attr_unrelated"))
    rss() - before < 50
}
print(calls(100))
print(attr(fs[[50]], "data")[1:3])
print(.Call("capi_attr_read", fs[[2]], "data")[1:3])
