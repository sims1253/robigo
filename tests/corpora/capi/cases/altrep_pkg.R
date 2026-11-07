dyn.load("altrep_pkg.so")
calls <- function() .Call("altrep_pkg_calls")

# Elt is called where GNU R reads elements through the class
x <- .Call("altrep_pkg_count_seq", 5L, NULL, 0L)
print(calls())
s <- 0L
for (v in x) s <- s + v
print(s)
print(calls())
print(x[[3L]])
print(calls())
print(unlist(lapply(x, function(e) e * 2L)))
print(calls())
print(vapply(x, function(e) e + 0.5, 0))
print(calls())
print(x[c(2L, 4L)])
print(calls())
f <- function(x) {
    s <- 0L
    for (v in x) s <- s + v
    s + x[[2L]]
}
print(f(x))
print(f(x))
print(calls())

# Sum and Max answer doubles, Min an integer; the hints answer without Elt
print(sum(x))
print(typeof(sum(x)))
print(typeof(max(x)))
print(typeof(min(x)))
print(typeof(sum(x, x)))
print(is.unsorted(x))
print(anyNA(x))
print(calls())

# R code in Elt, and an error in Elt
cb <- function(i) cat("elt", i, "\n")
y <- .Call("altrep_pkg_count_seq", 3L, cb, 0L)
for (v in y) cat("got", v, "\n")
z <- .Call("altrep_pkg_count_seq", 4L, NULL, 3L)
print(tryCatch({ for (v in z) cat("got", v, "\n"); "done" }, error = function(e) conditionMessage(e)))
print(tryCatch(z[[3L]], error = function(e) conditionMessage(e)))
print(tryCatch(lapply(z, function(e) e), error = function(e) conditionMessage(e)))
print(z[[4L]])
print(calls())

# a live logical: `if` reads it through Elt every time
t <- .Call("altrep_pkg_live")
.Call("altrep_pkg_set_flag", FALSE)
if (t) print("due") else print("not due")
.Call("altrep_pkg_set_flag", TRUE)
if (t) print("due") else print("not due")
g <- function(t) if (t) "due" else "not due"
print(g(t))
# `&&` and `||` read it through Elt too, interpreted and compiled
print(TRUE && t)
print(t || FALSE)
h <- function(t) c(TRUE && t, t || FALSE)
print(h(t))
.Call("altrep_pkg_set_flag", FALSE)
print(g(t))
print(TRUE && t)
print(t || FALSE)
print(h(t))
print(t[[1L]])

# a modified copy is an ordinary vector
x2 <- x
x2[1L] <- 10L
print(x2[[1L]])
print(calls())

# serialize v3 writes the class and its Serialized_state
r <- serialize(x, NULL, version = 3L)
print(r[-(1:23)])

# .Internal(inspect): the heap fields that are not facts of both
# implementations (address, generation, MARK, reference count) are masked;
# the strings and symbols are ones GNU R never hashed or bound in base
# (HASHASH, the string cache's bucket chain and the base symbol cache bits
# are GNU R's caches)
mask <- function(s) {
    s <- gsub("@[0-9a-f]+ ", "@ ", s)
    s <- gsub("g[0-9]c", "gc", s)
    s <- gsub("MARK,?", "", s)
    s <- gsub("REF\\([0-9]+\\),?", "", s)
    gsub(",\\]", "]", s)
}
ins <- function(x, ...) {
    tc <- textConnection("out", "w", local = TRUE)
    sink(tc)
    .Internal(inspect(x, ...))
    sink()
    close(tc)
    cat(mask(out), sep = "\n")
}
ins(x)
print(calls())
ins(NULL)
ins(c(1.5, NA, Inf, -2e10))
ins(c(TRUE, NA, FALSE))
ins(c(1L, 2L, 3L, 4L, 5L, 6L, 7L))
ins(c(1L, 2L, 3L, 4L, 5L, 6L, 7L), 0L, 3L)
ins(1:10)
ins(as.raw(c(1, 255)))
ins(structure(c(1L, 5L), foo = "zq7"))
ins(c("zq1", NA))
ins(list(1L, "zq2"))
ins(quote(foo(a, b)))
h <- function(a, b = 2) foo(a, b)
ins(h)
