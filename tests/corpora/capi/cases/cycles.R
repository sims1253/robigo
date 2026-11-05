dyn.load("cycles.so")

# A C finalizer runs at the first gc() after its pointer is unreachable,
# also when a reference cycle held it (an environment bound in itself, or
# a closure in the frame it was made in: test_that()'s environments).
local({
    h <- .Call("cycles_make", "self-bound")
    self <- environment()
})
f <- function() {
    h <- .Call("cycles_make", "closure-cycle")
    g <- function() h
    invisible(NULL)
}
f()
invisible(gc())
cat("after one gc()\n")
invisible(gc())
cat("after two gc()s\n")

# A closure only C keeps, made in a frame it is bound in, still works
# after a gc(): its environment is alive (curl's multi_add 'data').
make_callback <- function(k) {
    cb <- function(x) {
        x + k
    }
    .Call("cycles_hold", cb)
}
p <- make_callback(10)
invisible(gc())
print(.Call("cycles_call", p, 1))
invisible(gc())
print(.Call("cycles_call", p, 2))

# The same through a pointer that is itself only in a cycle-held frame.
q <- local({
    held <- make_callback(100)
    self <- environment()
    function(x) .Call("cycles_call", held, x)
})
invisible(gc())
print(q(1))

# With runtime weak references (an R finalizer on an environment, as
# testthat and withr register) proxies are marked by the runtime graph:
# the same callbacks and finalizers, in that mode.
invisible(reg.finalizer(new.env(), function(e) cat("R finalizer ran\n")))
invisible(gc())
r <- make_callback(1000)
invisible(gc())
print(.Call("cycles_call", r, 1))
add <- function(data) {
    con <- data
    data <- function(x) {
        if (!isOpen(con)) open(con, "wb")
        writeBin(x, con = con)
        length(rawConnectionValue(con))
    }
    .Call("cycles_hold", data)
}
t1 <- function() {
    con <- rawConnection(raw(0), "r+")
    on.exit(close(con))
    p <- add(con)
    invisible(gc())
    print(.Call("cycles_call", p, as.raw(1:3)))
}
t1()
local({
    h <- .Call("cycles_make", "weak-mode cycle")
    self <- environment()
})
invisible(NULL)  # .Last.value no longer holds local()'s environment
invisible(gc())
cat("after one more gc()\n")

# A cycle through both heaps: the frame holds the pool, the pool's
# protected field holds the callback, the callback's environment is the
# frame (curl's pools). One gc() collects it, as GNU R's one heap does.
pooled <- function(name) {
    cb <- function() name
    pool <- .Call("cycles_pool", name, cb)
    invisible(NULL)
}
pooled("a")
invisible(gc())
cat("after one gc() for pool a\n")

# kept alive from a global: nothing is collected until that goes
kept <- function(name) {
    cb <- function() name
    pool <- .Call("cycles_pool", name, cb)
    assign("keeper", cb, globalenv())
    invisible(NULL)
}
kept("b")
invisible(gc())
cat("pool b is kept:", keeper(), "\n")
rm(keeper)
invisible(gc())
cat("after keeper went\n")

# an R finalizer on the frame: it runs first, the frame stays alive for
# its round, and the pool goes with the frame at the next gc()
finalized <- function(name) {
    cb <- function() name
    pool <- .Call("cycles_pool", name, cb)
    reg.finalizer(environment(), function(e) cat("frame of", e$name, "finalized\n"))
    invisible(NULL)
}
finalized("c")
invisible(gc())
cat("after the frame's finalizer\n")
invisible(gc())
cat("after the next gc()\n")

# a finalizer that allocates, reusing the dead objects' slots
busy <- function() {
    cb <- function() 1
    pool <- .Call("cycles_busy_pool", cb)
    invisible(NULL)
}
busy()
invisible(gc())
again <- local({
    cb <- function(x) "still works"
    p <- .Call("cycles_hold", cb)
    function() .Call("cycles_call", p, NULL)
})
print(again())
invisible(gc())
print(again())
cat("after the busy finalizer\n")
