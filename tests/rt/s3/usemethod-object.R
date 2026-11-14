# UseMethod(generic, object): the method gets the generic's arguments, not the
# object alone; NextMethod appends the named arguments it adds to the call.
g <- function(x) UseMethod("g", x)
g.foo <- function(x) x
g(structure(1, class = "foo"))
h <- function(a, b = 2) UseMethod("h", b)
h.numeric <- function(a, b = 2) a + b
h(1)
h(1, 5)
t3 <- function(x, ...) UseMethod("t3"); t3.a <- function(x, ...) NextMethod(n = 3, m = 4); t3.default <- function(x) x
tryCatch(t3(structure(1, class = "a")), error = function(e) conditionMessage(e))
t3(structure(1, class = "a"))
