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
