events <- character()
add <- function(x) events <<- c(events, x)

# an unforced default-argument promise keeps the environment alive
make <- function() {
    e <- new.env()
    e$id <- "promise"
    reg.finalizer(e, function(e) add(e$id))
    function(x = e) x
}
f <- make()
invisible(gc())
cat("live:", length(events), "\n")
v <- f()
cat("value:", v$id, "\n")
rm(f, v)
invisible(gc())
cat("dead:", paste(events, collapse = ","), "\n")

# an argument promise keeps the environment alive while the call runs
events <- character()
e <- new.env()
e$id <- "argument"
reg.finalizer(e, function(e) add(e$id))
hold <- function(x) {
    invisible(gc())
    cat("frame live:", length(events), "\n")
    x$id
}
cat("frame value:", hold(e), "\n")
rm(e)
invisible(gc())
cat("frame dead:", paste(events, collapse = ","), "\n")

# an environment held only by a promise value that nothing references dies
events <- character()
e <- new.env()
e$id <- "forced"
reg.finalizer(e, function(e) add(e$id))
p <- (function(x) x)(e)
rm(e)
invisible(gc())
cat("forced live:", length(events), "\n")
rm(p)
invisible(gc())
cat("forced dead:", paste(events, collapse = ","), "\n")
