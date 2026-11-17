# traceback.R's traceback() over errors.c's .Traceback and
# .Internal(traceback(skip)): the calls of the last uncaught error, which
# catch.script.errors lets the script survive; an error caught by a handler
# leaves .Traceback as it was.
f <- function(x) stop("boom: ", x)
g <- function(y) f(y)
h <- function() g("deep")
options(catch.script.errors = TRUE)
h()
print(traceback())
x <- traceback(1)
print(x)
print(length(.Traceback))
k <- function() h()
tryCatch(k(), error = function(e) NULL)
print(length(traceback()))
