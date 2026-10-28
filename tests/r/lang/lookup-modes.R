x <- 7L
print(get("x", mode = "double"))
x <- 3.5
print(exists("x", mode = "integer"))
x <- factor("a")
print(exists("x", mode = "numeric"))
x <- sum
print(exists("x", mode = "closure"))
x <- function() 1
print(exists("x", mode = "builtin"))
probe <- function(expr) tryCatch(expr, error = function(e) {
    print(conditionMessage(e))
    print(conditionCall(e))
})
probe(exists("x", mode = "bogus"))
probe(exists(c("x", "y")))
probe(get("absent_lookup_binding"))
x <- "global"
f <- function() {
    x <- "local"
    print(get("x", envir = 0))
    print(get("x", envir = 1))
    print(get0("x", envir = -2))
}
f()
e <- new.env()
delayedAssign("x", stop("forced"), assign.env = e)
print(exists("x", e))
probe(get("x", e))
