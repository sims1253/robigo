# A cyclic environment graph is physically reclaimed: a finalizer sees its
# key, and repeated allocate/drop cycles keep RSS flat (memory.c's sweep).
rss <- function() {
    s <- readLines("/proc/self/statm")
    as.numeric(strsplit(s, " ")[[1]])[2] * 4096 / 1024
}
make <- function(n) {
    for (i in seq_len(n)) {
        a <- new.env(); b <- new.env(); c <- new.env()
        a$b <- b; b$c <- c; c$a <- a
        a$f <- function() a
        b$g <- function() b
    }
}
e <- new.env(); e$self <- e
seen <- FALSE
reg.finalizer(e, function(e) seen <<- TRUE)
rm(e)
invisible(gc())
cat(if (seen) "finalized\n" else "pending\n")
make(20000)
invisible(gc())
make(20000)
invisible(gc())
base <- rss()
for (k in 1:8) {
    make(20000)
    invisible(gc())
}
cat(if (rss() - base < 65536) "stable\n" else "grew\n")
