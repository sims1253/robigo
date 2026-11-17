# RNGkind's and set.seed's warnings and errors; invalid .Random.seed values
show <- function(expr) withCallingHandlers(tryCatch(expr,
    error = function(e) cat("Error:", conditionMessage(e), "\n")),
    warning = function(w) { cat("Warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") })
show(set.seed(1, kind = "Marsaglia-Multicarry"))
RNGkind()
show(RNGkind(normal.kind = "Kinderman-Ramage"))
show(RNGkind(normal.kind = "Ahrens-Dieter"))
show(RNGkind("default", "default"))
show(set.seed(NA))
show(set.seed(NA_integer_))
show(set.seed("a"))
show(.Internal(RNGkind(99L, NULL, NULL)))
show(.Internal(RNGkind(NULL, 9L, NULL)))
show(.Internal(RNGkind(NULL, -5L, NULL)))
show(.Internal(RNGkind(NULL, NULL, 3L)))
show(.Internal(RNGkind(-1L, -1L, -1L)))
RNGkind()
show(.Internal(set.seed(1, 99L, NULL, NULL)))
show(RNGkind("foo"))
# an invalid .Random.seed is replaced (with a time seed) after a warning
check <- function(v) {
    .Random.seed <<- v
    show(RNGkind())
    cat(exists(".Random.seed"), typeof(.Random.seed), length(.Random.seed), .Random.seed[1], "\n")
}
check(1.5)
check("x")
check(c(NA_integer_, 1L))
check(c(-1L, 1L))
check(c(11001L, 1L))
check(c(700L, 1L))
check(c(20003L, 1L))
check(c(5L, 1L))
check(c(8L, 1L))
check(c(9L, 1L))
.Random.seed <- c(403L, 1L, 2L)
show(RNGkind("Wich"))
.Random.seed <- 3L
show(RNGkind("Wich"))
cat(length(.Random.seed), .Random.seed[1], "\n")
# a .Random.seed with attributes is replaced, not written in place
set.seed(5)
.Random.seed <- structure(.Random.seed, foo = 1)
set.seed(6)
attributes(.Random.seed)
