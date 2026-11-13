eval(quote(1 + 2))
x <- 5
eval(quote(x * 2), list(x = 10))
eval(quote(x * 2), pairlist(x = 3))
f <- function() { eval(quote(return(7))); 99 }
f()
e <- new.env()
eval(quote(y <- 3), e)
eval(quote(y), e)
g <- function() eval(quote(sys.call()))
g()
h <- function(a) eval(quote(a), 1)
eval(1)
eval("text")
