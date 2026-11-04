# Native call trees that assign elements (`x[i] <- v`, `x[[i]] <- v`) and
# compute with `abs`/`sqrt` of values of any type, also called per element
# by lapply().

bounce <- function(ball) {
    ball[1] <- ball[1] + ball[3]
    ball[2] <- ball[2] + ball[4]
    bounced <- FALSE
    if (ball[1] > 500) {
        ball[1] <- 500
        ball[3] <- 0 - abs(ball[3])
        bounced <- TRUE
    }
    if (ball[1] < 0) {
        ball[1] <- 0
        ball[3] <- abs(ball[3])
        bounced <- TRUE
    }
    if (ball[2] > 500) {
        ball[2] <- 500
        ball[4] <- 0 - abs(ball[4])
        bounced <- TRUE
    }
    if (ball[2] < 0) {
        ball[2] <- 0
        ball[4] <- abs(ball[4])
        bounced <- TRUE
    }
    list(ball, bounced)
}
bounces <- 0
step <- function(b) {
    r <- bounce(b)
    if (r[[2]]) bounces <<- bounces + 1
    r[[1]]
}
balls <- lapply(1:60, function(k) c(k * 37 %% 500, k * 91 %% 500, k %% 7 * 20 - 60, k %% 5 * 30 - 60))
orig <- balls
for (i in 1:40) balls <- lapply(balls, step)
bounces
balls[[1]]
balls[[60]]
sum(unlist(balls))
# The caller's vectors are not modified.
identical(orig, lapply(1:60, function(k) c(k * 37 %% 500, k * 91 %% 500, k %% 7 * 20 - 60, k %% 5 * 30 - 60)))

# Assigning elements of locals and formals.
grow <- function(v, k) {
    v[k] <- v[k] * 2
    v[[1]] <- -1
    v
}
x <- c(1, 2, 3, 4)
for (i in 1:600) y <- grow(x, 1 + i %% 4)
x
y
w <- c(a = 1, b = 2, c = 3)
for (i in 1:600) y <- grow(w, 2)
y
# What leaves native code: appending, coercion, out of range, odd indices.
for (i in 1:600) y <- grow(x, 5)
y
for (i in 1:600) y <- grow(1:4, 2)
y
storage.mode(y)
for (i in 1:600) y <- grow(x, 7)
y
for (i in 1:600) y <- grow(x, 2.9)
y
tryCatch(grow(x, 0), error = function(e) conditionMessage(e))
tryCatch(grow(x, NA), error = function(e) conditionMessage(e))
tryCatch(grow(x, NA_real_), error = function(e) conditionMessage(e))
y <- grow(x, -1)
y
cls <- structure(c(1, 2, 3), class = "myclass")
`[<-.myclass` <- function(x, i, value) {
    cat("[<-.myclass\n")
    structure(unclass(x), class = "myclass")
}
y <- grow(cls, 1)
unclass(y)

# Lists.
setl <- function(l, i, v) {
    l[[i]] <- v
    l
}
l <- list(1, "a", TRUE)
for (i in 1:600) r <- setl(l, 2, i)
str2 <- function(x) cat(paste(sapply(x, function(e) paste(class(e), format(e))), collapse = " | "), "\n")
str2(r)
str2(l)
for (i in 1:600) r <- setl(l, 4, "new")
str2(r)
r <- setl(l, 2, NULL)
str2(r)
length(r)
r <- setl(l, 6, 0)
length(r)
for (i in 1:600) r <- setl(l, 1, list(1, 2))
str2(r[[1]])

# A local built up and modified in place.
build <- function(n) {
    out <- c(0, 0, 0, 0, 0)
    for (k in 1:5) out[k] <- k * n
    out[[3]] <- sqrt(out[[3]])
    out
}
for (i in 1:600) z <- build(i)
z
w2 <- function(x) sqrt(x[[1]])
for (i in 1:600) z <- w2(list(4))
z
z <- w2(list(-4))
z

# lapply calling native code: errors and conditions part way, names,
# sys.call(), the counter written back after each element.
n <- 0
cnt <- function(v) {
    v[1] <- v[1] + 1
    n <<- n + 1
    v
}
xs <- rep(list(c(1, 2)), 700)
names(xs) <- paste0("e", seq_along(xs))
r <- lapply(xs, cnt)
n
r$e700
xs[[650]] <- "oops"
r <- tryCatch(lapply(xs, cnt), error = function(e) conditionMessage(e))
r
n
who <- function(v) {
    v[1] <- 0
    if (length(v) > 2) print(sys.call())
    v
}
xs <- rep(list(c(1, 2)), 700)
xs[[690]] <- c(1, 2, 3)
r <- lapply(xs, who)
r[[690]]
