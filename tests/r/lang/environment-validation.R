probe <- function(expr) tryCatch(expr, error = function(e) {
    print(conditionMessage(e))
    print(conditionCall(e))
})
f <- function() 1
probe(environment(f) <- NULL)
probe(environment(f) <- 3)
x <- NULL
probe(environment(x) <- new.env())
probe(eval(quote(1), c(0, 1)))
print(identical(as.environment(length(search()) + 1), emptyenv()))
e <- new.env()
e$x <- 1
lockEnvironment(e)
probe(rm("x", envir = e))
print(e$x)
probe(rm("absent", envir = baseenv()))
