# Compact sequences (GNU R altclasses.c compact_intseq/compact_realseq)
# hold no memory for their elements (issue #460). serialize() writes them
# in ALTREP form, reading, sum() and for need no expansion, and modifying
# one expands it. An operation whose result cannot be allocated fails with
# allocVector's catchable error, at the result's size.
bytes <- function(x) paste(as.integer(serialize(x, NULL, version = 3)), collapse = ",")
cat(bytes(1:2147483649), "\n")
cat(length(serialize(1:2147483649, NULL, version = 3)), "\n")
cat(bytes(1:10), "\n")
cat(bytes(10:1), "\n")
cat(bytes(-3:-2147483650), "\n")
cat(bytes(as.numeric(3:1)), "\n")
u <- unserialize(serialize(1:2147483649, NULL, version = 3))
print(length(u))
print(u[2147483649])

# modifying a sequence expands a copy; the original is unchanged
x <- 1:10
x[3] <- 5L
cat(bytes(x), "\n")
print(x)
y <- 1:10
z <- y
z[[2]] <- 0L
print(y)
print(z)

# sum: GNU R's closed forms (an integer while it fits)
print(sum(1:1e11))
print(sum(-5:5))
print(sum(1:65536))
print(typeof(sum(1:65536)))
print(typeof(sum(1:65535)))

# reading without expanding, at top level and in a (compiled) function
r <- 1:1e11
print(r[1e11 - 0:2])
for (i in 2147483640:2147483650) NULL
print(i)
g <- function() {
    r <- 1:1e11
    print(r[1e11 - 0:2])
    print(r[3])
    print(r[[4]])
    print(r[5:7])
    print(r[7:5])
    i <- 8:9
    print(r[i])
    s <- 0
    for (k in 1:5) s <- s + r[k]
    print(s)
    print(length(r))
    print(sum(r))
}
g()
h <- function() {
    r <- 1:10
    r[2:3] <- 0L
    print(r)
    q <- 1:10
    q[[4]] <- 99L
    print(q)
    w <- 1:10
    w[c(1, 10)] <- 0L
    print(w)
    v <- 1:10
    v[11] <- 11L
    print(v)
}
h()

# results too large to allocate
try_it <- function(expr) cat(tryCatch({ expr; "allocated" }, error = function(e) conditionMessage(e)), "\n")
k <- function() {
    x <- 0:(1e15 - 1)
    try_it(x * 2)
    try_it(x + 1L)
    try_it(-x)
    try_it(x == 1)
    try_it(as.integer(x))
    try_it(c(x, 1))
    try_it(rev(x))
    try_it(x[-1])
    try_it(sqrt(x))
    try_it(x %% 2)
    try_it(!x)
    try_it(is.na(x))
    try_it(cumsum(x))
    try_it(x[2:1e14])
    try_it({ y <- x; y[1] <- 0 })
    try_it({ y <- x; y[[2]] <- 0 })
    try_it({ y <- x; y[2:3] <- 0 })
    try_it(round(x))
    try_it(diff(x))
    try_it(unique(x))
    try_it(x[x > 0])
    try_it(rep(x, 2))
    print(x[3:5])
    print(x[[1e15]] == 1e15 - 1)
    print(length(x))
}
k()
x <- 0:(1e15 - 1)
try_it(x * 2)
try_it(-x)
try_it(x[-1])
try_it(rev(x))
try_it({ x[1] <- 0 })
try_it(c(x, 1))
print(x[3:5])

# as.double keeps a sequence of its type (ascommon) and turns a compact
# integer sequence into a compact real one (compact_intseq_Coerce)
v <- as.numeric(1:2147483649)
print(length(v))
print(v[2147483649])
cat(length(serialize(v, NULL, version = 3)), "\n")
w <- as.double(-1:-2000000)
print(typeof(w))
print(w[2e6])
cat(bytes(w), "\n")
y <- as.integer(1:2147483647)
print(y[2147483647])
print(sum(y))
