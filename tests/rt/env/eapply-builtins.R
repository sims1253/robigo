# envir.c do_eapply and do_builtins.
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  if (inherits(r, "error")) {
    cat("error:", conditionMessage(r), "| call:", deparse(conditionCall(r)), "\n")
  } else str1(r)
  invisible()
}
str1 <- function(v) {
  if (is.list(v)) {
    cat("list of", length(v), if (!is.null(names(v))) paste("names:", paste(names(v), collapse = ",")), "\n")
    for (i in seq_along(v)) { cat(" ", i, ": "); str1(v[[i]]) }
  } else cat(typeof(v), ":", v, "\n")
}
# an unhashed environment: its frame's order, as as.list(env)
e <- new.env(hash = FALSE)
assign("a", 1, envir = e); assign("b", 2:3, envir = e); assign(".h", "hidden", envir = e)
show(eapply(e, length))
show(eapply(e, function(x) x * 10))
show(eapply(e, length, all.names = TRUE))
show(eapply(e, length, USE.NAMES = FALSE))
show(eapply(e, function(x, y) x + y, y = 100))
identical(names(eapply(e, identity)), names(as.list(e)))
# a hashed environment: the table's order
h <- new.env(hash = TRUE)
for (n in c("p", "q", "r", "s")) assign(n, nchar(n), envir = h)
identical(names(eapply(h, identity)), names(as.list(h)))
show(eapply(h, function(x) x))
# promises are forced; an active binding is called
d <- new.env(hash = FALSE)
delayedAssign("lazy", { cat("forced\n"); 42 }, assign.env = d)
makeActiveBinding("act", function() "active", d)
show(eapply(d, identity))
# a function's frame, X and i in the caller
f <- function() { u <- 1; v <- 2; eapply(environment(), function(x) x + 1) }
show(f())
g <- function(env) eapply(env, function(x) deparse(sys.call()))
show(g(e))
# FUN by name; errors
show(eapply(e, "length"))
show(eapply(NULL, length))
show(eapply(list(), length))
show(eapply(emptyenv(), length))
show(.Internal(eapply(e, function(x) x, FALSE, TRUE)))
show(.Internal(eapply(e, length, NA, NA)))
# builtins: base's bound names, or the .Internal ones, sorted decreasing
b <- builtins()
bi <- builtins(TRUE)
length(b); length(bi)
b[1:3]; b[length(b) - 2:0]
bi[1:3]; bi[length(bi) - 2:0]
identical(b, sort(b, decreasing = TRUE))
c("aperm", "Recall", "sum", "eapply") %in% bi
c(".C", "sum", "aperm", "nope") %in% b
identical(builtins(NA), b)
show(.Internal(builtins()))
