# Native loops calling closures that read and `<<-`-assign variables of
# their enclosing environments (the loop's cells), as in the shootout's
# fasta: `for (i in 1:n) rs[[i]] <- myrandom(1)`.

# A generator in the global environment, read-only globals beside it.
last <- 42L
scale <- 4095
gen <- function() {
    last <<- (last * 3877L + 29573L) %% 139968L
    last * (scale / 139968)
}
rs <- double(2000)
for (i in 1:2000) rs[[i]] <- gen()
print(c(last, sum(rs), rs[c(1, 2, 1000, 2000)]))

# With an argument, called from a function's loop, twice.
seed <- 7L
rnd <- function(m) {
    seed <<- (seed * 3877L + 29573L) %% 139968L
    m * seed / 139968
}
fill <- function(n) {
    x <- double(n)
    for (i in 1:n) x[[i]] <- rnd(10)
    x
}
a <- fill(700)
b <- fill(700)
print(c(seed, sum(a), sum(b)))

# A helper defined in the function, counting in its frame: the pending
# assignment before the loop, and the value read after it.
counter <- function(n) {
    k <- 0L
    tick <- function() k <<- k + 1L
    for (i in 1:n) tick()
    k
}
print(counter(1500))
counter2 <- function(n) {
    total <- 0
    add <- function(v) total <<- total + v
    for (i in 1:n) add(i * 0.5)
    c(total, total * 2)
}
print(counter2(1000))

# Two helpers sharing a variable.
s <- 0
up <- function() s <<- s + 2
down <- function() s <<- s - 1
for (i in 1:1000) {
    up()
    down()
}
print(s)

# Integer overflow inside the helper midway: the warning, and the values
# before and after.
big <- .Machine$integer.max - 600L
inc <- function() big <<- big + 1L
r <- integer(1000)
for (i in 1:1000) r[[i]] <- inc()
print(c(big, r[600], r[601], sum(is.na(r))))

# NA in a condition inside the loop, after some calls.
w <- 0L
bump <- function() w <<- w + 1L
v <- c(rep(1, 700), NA)
res <- tryCatch({
    for (i in 1:701) {
        bump()
        if (v[[i]] > 0) next
    }
    "done"
}, error = function(e) conditionMessage(e))
print(c(res, w))

# A type change: an integer cell assigned a double.
z <- 1L
half <- function() z <<- z + 0.5
for (i in 1:1000) half()
print(c(z, typeof(z)))

# The formal named like the variable assigned.
q <- 1
same <- function(q) {
    q <<- q + 1
    q
}
o <- double(800)
for (i in 1:800) o[[i]] <- same(i)
print(c(q, sum(o)))

# A locked binding.
lk <- 0
lockBinding("lk", globalenv())
lkf <- function() lk <<- lk + 1
print(tryCatch({
    for (i in 1:1000) lkf()
    "no error"
}, error = function(e) conditionMessage(e)))
unlockBinding("lk", globalenv())
print(lk)

# An active binding read by the helper.
cnt <- 0
makeActiveBinding("act", function() {
    cnt <<- cnt + 1
    3
}, globalenv())
ra <- function() act * 2
x <- double(600)
for (i in 1:600) x[[i]] <- ra()
print(c(sum(x), cnt > 0))

# The loop names the variable itself, before and after the call.
y <- 0
yinc <- function() y <<- y + 1
u <- double(1000)
for (i in 1:1000) {
    yinc()
    u[[i]] <- y
}
print(c(y, sum(u)))

# The variable not bound yet: `<<-` creates it in the global environment.
newf <- function() fresh <<- 5
for (i in 1:600) newf()
print(fresh)

# Enclosing variables that are not scalars.
vec <- c(1, 2)
vf <- function() vec <<- vec + 1
for (i in 1:600) vf()
print(vec)

# A grandparent's variable, through two levels of functions.
outer_f <- function() {
    g <- 10L
    mid <- function() {
        h <- function() g <<- g + 1L
        for (i in 1:900) h()
        g
    }
    c(mid(), g)
}
print(outer_f())

# break out of the loop after the call.
br <- 0
brf <- function() br <<- br + 1
for (i in 1:1000) {
    brf()
    if (i >= 777L) break
}
print(br)

# A local variable of the same name in the caller's frame: the helper still
# assigns the global one.
shadow <- 100
sh <- function() shadow <<- shadow + 1
caller <- function() {
    shadow <- 0
    for (i in 1:700) sh()
    shadow
}
print(c(caller(), shadow))
