dyn.load("altrep_lazy_string.so")
lazy <- function(fn) .Call("lazy_new", fn)
mat <- function(x) .Call("lazy_is_materialized", x)
counts <- function() .Call("lazy_counts")

# dplyr's test-generics (#6525): lazy row names go through R's attribute
# work and C's raw attribute get/set unread
df <- data.frame(x = 1)
attrs <- attributes(df)
attrs$row.names <- lazy(function() "a")
attrs <- as.pairlist(attrs)
df2 <- .Call("lazy_set_attributes", df, attrs)
x <- .Call("lazy_reconstruct", df2, df)
print(mat(.Call("lazy_attributes", df2)$row.names))
print(mat(.Call("lazy_attributes", x)$row.names))
print(mat(.row_names_info(df2, 0L)))
print(counts())

# what GNU R does with a vector without reading it
z <- lazy(function() c("p", "q"))
print(typeof(z))
print(is.character(z))
print(class(z))
print(inherits(z, "character"))
print(identical(z, z))
l <- list(z, 1)
print(mat(l[[1L]]))
print(is.null(attributes(z)))
obj <- c(1L, 2L)
attr(obj, "lz") <- z
print(mat(attr(obj, "lz")))
f <- function(v) mat(v)
print(f(z))
print(counts())

# using it reads it
print(length(z))
print(mat(z))
print(counts()[["Length"]] > 0L)
print(z)
print(.row_names_info(df2, 2L))

# a class whose materialization runs R code or raises: the output and the
# condition come when R uses the vector, not before
w <- lazy(function() {
    cat("materializing\n")
    "w"
})
l2 <- list(w)
cat("before\n")
print(nchar(l2[[1L]]))
e <- lazy(function() stop("boom"))
l3 <- list(e)
print(tryCatch(length(l3[[1L]]), error = function(err) conditionMessage(err)))
print(mat(e))

# Deferred contents are read by top-level printing and index operations.
z <- lazy(function() c("p", "q"))
l <- list(z, 1)
l[[1L]]
v <- c(p = 1, q = 2)
print(v[lazy(function() "q")])
print(v[[lazy(function() "p")]])
v[lazy(function() "q")] <- 3
v[[lazy(function() "p")]] <- 4
print(v)
print(unserialize(serialize(list(lazy(function() c("a", "b"))), NULL)))
x <- 1L
attr(x, "lazy") <- lazy(function() c("a", "b"))
print(unserialize(serialize(x, NULL)))
print(do.call("$<-", list(list(), lazy(function() "p"), 1L)))
v <- c(p = 1, q = 2)
print(tryCatch({ v[lazy(function() stop("index boom"))] <- 9; NULL },
               error = function(e) conditionMessage(e)))
print(v)
print(tryCatch({ v[[lazy(function() stop("index boom"))]] <- 9; NULL },
               error = function(e) conditionMessage(e)))
print(v)
