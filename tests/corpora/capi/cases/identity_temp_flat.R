dyn.load("identity.so")

rss <- function() {
    line <- grep("^VmRSS:", readLines("/proc/self/status"), value = TRUE)
    as.numeric(gsub("[^0-9]", "", line)) / 1024
}
for (i in 1:1000) a <- .Call("capi_id_addr", c(i, 1))
invisible(gc())
before <- rss()
for (i in 1:100000) a <- .Call("capi_id_addr", c(i, 1))
x <- c(1, 2)
for (i in 1:100000) a <- .Call("capi_id_addr", x)
invisible(gc())
print(rss() - before < 50)
print(nchar(a) > 0)
