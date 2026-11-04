# x[i] and x[i] <- v with index vectors in compiled code (the VM's
# element-by-element paths and everything they leave to `[` and `[<-`).
show <- function(x) cat(deparse(x), "\n")
get <- function(x, i) x[i]
set <- function(x, i, v) {
    x[i] <- v
    x
}
for (k in 1:2) {
    show(get(c(10, 20, 30, 40), c(4L, 1L, 1L)))
    show(get(c(10, 20, 30, 40), c(2.9, 1.1)))
    show(get(1:5, c(5L, 3L)))
    show(get(c(TRUE, NA, FALSE), c(2L, 3L)))
    show(get(c("a", NA, "c"), c(3, 2, 1)))
    show(get(c(10, 20), integer()))
    show(get(c(10, 20), c(1L, NA)))
    show(get(c(10, 20), c(1, NaN)))
    show(get(c(10, 20), c(0L, 2L)))
    show(get(c(10, 20), c(-1L)))
    show(get(c(10, 20), c(3L, 1L)))
    show(get(c(10, 20), c(2.5, 3.5)))
    show(get(c(a = 1, b = 2), c(2L, 1L)))
    show(get(matrix(1:4, 2), c(4L, 1L)))
    show(get(factor(c("u", "v")), c(2L, 2L)))
    show(get(list(1, "a"), c(2L, 1L)))
    show(get(c(10, 20, 30), c(TRUE, FALSE)))
    show(get(c(10, 20, 30), structure(c(1L, 2L), names = c("p", "q"))))
    show(get(c(10, 20, 30), factor(c("b", "a"))))

    show(set(c(1, 2, 3, 4), c(4L, 2L), c(40, 20)))
    show(set(c(1, 2, 3, 4), c(1, 3.7), 0))
    show(set(c(1, 2, 3, 4), 1:4, c(-1, -2)))
    show(set(1:4, c(2L, 3L), c(20L, 30L)))
    show(set(1:4, c(2L, 3L), c(2.5, 3.5)))
    show(set(1:4, c(2L, 3L), c(TRUE, NA)))
    show(set(c(1, 2), c(1L, 2L), c(NA, 1L)))
    show(set(c(TRUE, FALSE), c(2L, 1L), c(NA, TRUE)))
    show(set(c(TRUE, FALSE), c(2L, 1L), 1:2))
    show(set(c("a", "b", "c"), c(3L, 1L), c("z", NA)))
    show(set(c("a", "b", "c"), c(3L, 1L), 1:2))
    show(set(c(1, 2, 3), c(1L, 1L, 2L), c(7, 8, 9)))
    show(set(c(1, 2, 3), integer(), 5))
    show(set(c(1, 2, 3), c(1L, 5L), c(7, 8)))
    show(set(c(1, 2, 3), c(1L, NA), 7))
    show(set(c(1, 2, 3), c(0L, 1L), 7))
    show(set(c(1, 2, 3), c(-1L), 7))
    show(set(c(a = 1, b = 2), c(2L, 1L), c(5, 6)))
    show(set(matrix(1:4, 2), c(4L, 1L), c(0L, 9L)))
    show(set(c(1, 2, 3), c(1L, 2L), c(x = 5, y = 6)))
    show(set(list(1, 2), c(2L, 1L), list("a", "b")))
    print(tryCatch(set(c(1, 2, 3), c(1L, 2L), numeric()), error = function(e) conditionMessage(e)))
    print(withCallingHandlers(set(c(1, 2, 3), c(1L, 2L, 3L), c(5, 6)),
                              warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }))
}

# Copy-on-modify, the vector itself as index or value, and a vector
# inherited from the global environment.
shared <- function() {
    a <- c(1, 2, 3)
    b <- a
    b[c(1L, 3L)] <- c(9, 9)
    a[a] <- a * 10
    list(a, b)
}
show(shared())
glob <- c(5, 6, 7)
loc <- function() {
    glob[c(1L, 2L)] <- c(0, 0)
    glob
}
show(loc())
show(glob)

# Lazy arguments as indices, and in loops (as in the shootout's fasta_3).
lazy <- function(x, i) x[i]
show(lazy(c(4, 5, 6), { cat("forced\n"); c(3L, 1L) }))
bsearch <- function(psum, rs) {
    n <- length(psum)
    line <- length(rs)
    inds <- 1:line
    lo <- rep.int(1L, line)
    hi <- rep.int(n, line)
    mid <- integer(line)
    repeat {
        mid[inds] <- (lo[inds] + hi[inds] - 1L) %/% 2
        ge <- which(psum[mid] >= rs)
        hi[ge] <- mid[ge]
        lt <- which(psum[mid] < rs)
        lo[lt] <- mid[lt] + 1L
        inds <- which(lo < hi)
        if (!length(inds)) break
    }
    list(hi, typeof(hi), typeof(mid))
}
ps <- cumsum(c(0.27, 0.12, 0.12, 0.27, rep(0.02, 11)))
for (k in 1:3) show(bsearch(ps, c(0.1, 0.5, 0.99, 0.3, 0.75, 0.0, 1)))
