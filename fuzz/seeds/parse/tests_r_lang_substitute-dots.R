f <- function(...) {
    print(substitute(...))
    print(substitute(...()))
    print(typeof(substitute(...())))
    print(substitute(list(...)))
}
f(x + y, z + w)
f(first = a, second = b)
f()
print(substitute(...()))
print(substitute(list(...), list(... = NULL)))
tryCatch(substitute(list(...), list(... = 1)), error = function(e) {
    print(conditionMessage(e))
    print(conditionCall(e))
})
Ops.one <- function(a, b) list(.Method, NextMethod())
Ops.two <- function(a, b) .Method
x <- structure(1, class = c("one", "two"))
print(x == 1)
print(1 == x)
print(x == x)
