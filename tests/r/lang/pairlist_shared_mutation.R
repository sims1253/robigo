# C's SETCAR/SETCDR mutate a cons cell in place; R-level replacement
# primitives duplicate a shared pairlist or call first, so an alias keeps
# its cells. (The in-place half is exercised by the capi corpus's
# pairlist_alias case, which calls SETCAR/SETCDR/SET_TAG through .Call.)

# attr(x, "a") <- v on a shared pairlist: only the assignment target changes.
p <- pairlist(1, 2)
q <- p
attr(p, "a") <- 1
cat("p attrs:", names(attributes(p)), "\n")
cat("q attrs:", is.null(attributes(q)), "\n")

# The same for a shared language object.
f <- quote(a + b)
g <- f
attr(f, "tag") <- "yes"
cat("f tag:", attr(f, "tag"), " g tag:", is.null(attr(g, "tag")), "\n")

# [[<- on a shared pairlist and call.
p <- pairlist(1, 2)
q <- p
p[[1]] <- 9L
print(q)
f <- quote(a + b)
g <- f
f[[2]] <- 7L
print(f)
print(g)

# names<- writes the cells' tags, not an attribute: the alias keeps its tags.
p <- pairlist(x = 1, y = 2)
q <- p
names(p) <- c("u", "v")
print(names(p))
print(names(q))
f <- quote(fun(a, b))
g <- f
names(f) <- c("", "left", "right")
print(f)
print(g)

# class<- and unclass() on a shared pairlist.
p <- pairlist(1, 2)
q <- p
class(p) <- "mine"
cat("p class:", class(p), " q class attr:", is.null(attr(q, "class")), "\n")
p <- structure(pairlist(1, 2), class = "mine")
q <- p
u <- unclass(p)
cat("unclass copy:", class(u), " p class:", class(p), "\n")

# Body and formals of a closure: formals() is the closure's own pairlist,
# and the replacement primitives copy it before changing a tag.
mk <- function() function(a, b) a + b
clo <- mk()
fl <- formals(clo)
names(fl) <- c("left", "right")
print(formals(clo))

# A fresh literal held only by a promise: GNU R's MAYBE_REFERENCED sees the
# promise's reference, so attr<-, dimnames<- and setEncoding still copy.
delayedAssign("dp", pairlist(1, 2))
x <- dp
attr(x, "a") <- 1
cat("promise pairlist copied:", is.null(attr(dp, "a")), "\n")
delayedAssign("dm", matrix(1:4, 2))
m <- dm
dimnames(m) <- list(c("r1", "r2"), c("c1", "c2"))
cat("promise matrix copied:", is.null(dimnames(dm)), "\n")
delayedAssign("ds", "\u00e9")
s <- ds
Encoding(s) <- "bytes"
cat("promise string copied:", Encoding(ds), Encoding(s), "\n")

# Inside a function the argument promise is the only owner, and GNU R's
# tracemem() shows that dimnames<- and attr<- mutate it without a copy.
# No R-level alias can observe that either way: the function returns the
# same (mutated) value, which is what robigo's conservative Rc count gives
# too. (robigo does not report tracemem copies; see docs/ledger/core.md.)
f <- function(m) {
    dimnames(m) <- list(c("a", "b"), c("c", "d"))
    m
}
print(f(matrix(1:4, 2)))
g <- function(p) {
    attr(p, "a") <- 1
    p
}
print(attributes(g(pairlist(1, 2))))
