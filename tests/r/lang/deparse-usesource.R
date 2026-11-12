# deparse(), dput() and dump() with useSource print a closure's source text
# (comments and layout kept) from its srcref; R: deparse.c:src2buff1.
src <- "function(x, y = 2) {\n  # keep me\n  x +   y  # sum\n}"
f <- eval(parse(text = src, keep.source = TRUE))
print(deparse(f, control = "useSource"))
dput(f, control = "useSource")
print(deparse(f, control = "all"))
# the default control deparses the code
print(deparse(f))
dput(f)
dump("f", "")

# a closure without a srcref deparses its code
g <- eval(parse(text = src, keep.source = FALSE))
print(deparse(g, control = "useSource"))
h <- f
attr(h, "srcref") <- NULL
dput(h, control = "useSource")

# closures inside lists, with names and attributes
l <- list(a = f, b = 1L, g)
dput(l, control = "useSource")
dput(l, control = "all")
fa <- f
attr(fa, "note") <- "hi"
dput(fa, control = "all")
dput(fa, control = c("useSource", "niceNames"))

# a closure defined inside another keeps its own source
outer <- eval(parse(text = "function() {\n  function(z) z  * 2 # inner\n}", keep.source = TRUE))
inner <- outer()
dput(inner, control = "useSource")
dput(outer, control = "useSource")

# expression vectors print each element's source
e <- parse(text = "1 +   2 # c\nfoo( x,\n  y )", keep.source = TRUE)
print(deparse(e, control = "useSource"))
print(deparse(e))
e2 <- e
attr(e2, "srcref")[1] <- list(NULL)
print(deparse(e2, control = "useSource"))
e3 <- e
attr(e3, "srcref") <- attr(e, "srcref")[1]
print(deparse(e3, control = "useSource"))
print(deparse(e, control = "useSource", nlines = 2))
print(deparse(e, width.cutoff = 20, control = "useSource"))

# a list with a srcref list attribute uses it too
ll <- list(1, 2)
attr(ll, "srcref") <- attr(e, "srcref")
print(deparse(ll, control = "useSource"))

# the srcref goes through as.character(): any value works
k <- function(x) x
attr(k, "srcref") <- c("my", "own", "source")
print(deparse(k, control = "useSource"))
attr(k, "srcref") <- character()
print(deparse(k, control = "useSource"))
attr(k, "srcref") <- 1:3
print(deparse(k, control = "useSource"))
attr(k, "srcref") <- structure(1, class = "weirdsrc")
as.character.weirdsrc <- function(x, ...) 42L
r <- tryCatch(deparse(k, control = "useSource"), error = function(e) conditionMessage(e))
print(r)

# a function call whose fourth element is a character vector
q <- quote(function(x) x)
q[[4]] <- c("function(x)  # hi", NA, "  x")
print(deparse(q, control = "useSource"))
print(deparse(q))
q[[4]] <- 1:2
print(deparse(q, control = "useSource"))
