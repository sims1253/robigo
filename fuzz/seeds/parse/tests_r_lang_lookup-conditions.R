probe <- function(expr) tryCatch(expr, error = function(e) {
    print(class(e))
    print(conditionMessage(e))
    print(conditionCall(e))
})
e <- (function(x) environment())()
probe(get("x", e))
probe(get0("x", e))
probe(get(e$x))
f <- function(x) probe(x)
f()
f <- function(...) ..1
probe(f())
f <- function(a) a
probe(f(1, ))
marker <- "global"
f <- function() {
    marker <- "local"
    print(eval(quote(marker), -1))
}
f()
