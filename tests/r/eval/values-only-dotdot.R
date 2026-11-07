show_error <- function(expr) tryCatch(expr, error = function(e) cat(conditionMessage(e), "\n"))
g <- function() {
    assign("..1", 5L)
    eval(quote(length(..1)))
}
show_error(g())
h <- function(...) {
    assign("..1", 5L)
    eval(quote(length(..1)))
}
print(h(1:3))
k <- function(...) {
    assign("..2", 5L)
    eval(quote(sum(..2)))
}
print(k(1L, 3:4))
show_error(k(1L))
e <- new.env(hash = TRUE)
assign("..1", 5L, e)
print(get("..1", e))
s <- as.name("..2")
stopifnot(identical(s, unserialize(serialize(s, NULL))))
h2 <- unserialize(serialize(h, NULL))
print(h2(1:4))
